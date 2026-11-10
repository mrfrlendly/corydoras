//! Importers: the images of an Are.na channel or a Pinterest board become
//! references of a language.
//!
//! - Are.na: the public channel API, every block with an image.
//! - Pinterest: the public RSS feed of a board. Pinterest puts only the
//!   newest 25 pins in the feed, and a private board has no feed.
//!
//! The browser extension is the other importer: see `extension/` and the
//! local bridge in the app.

use crate::library::Library;
use crate::util::fnv1a64;
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

/// Most images that one import takes.
pub const MAX_IMAGES: usize = 300;
const ARENA_PAGE: usize = 100;
const MAX_IMAGE_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BoardItem {
    /// The image to download.
    pub image_url: String,
    /// Another size of the same image, when the first one fails.
    pub fallback_url: Option<String>,
    pub title: String,
    /// The page that the image belongs to. It becomes the source of the item.
    pub source: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Board {
    /// "Are.na" or "Pinterest"
    pub service: String,
    pub title: String,
    pub url: String,
    pub items: Vec<BoardItem>,
    /// Blocks without an image (text, channels).
    pub skipped: usize,
    /// A limit of the service that the user must know.
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ImportSummary {
    pub added: usize,
    /// Images that the language had already.
    pub duplicates: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum BoardRef {
    Arena { slug: String },
    /// Without a board: the newest pins of the user.
    Pinterest { user: String, board: Option<String> },
}

/// Reads the service and the board from a link.
pub fn parse_board_url(url: &str) -> Result<BoardRef> {
    let u = url.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")).ok_or_else(|| anyhow!("Paste the link of the board, starting with https://."))?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let host = parts.next().unwrap_or("").to_lowercase();
    let segs: Vec<&str> = parts.collect();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if host == "are.na" {
        // are.na/<user>/<channel> or are.na/channel/<channel>
        return match segs.as_slice() {
            [first, slug, ..] if *first != "block" => Ok(BoardRef::Arena { slug: slug.to_string() }),
            _ => bail!("This is not the link of an Are.na channel. It looks like https://www.are.na/name/channel-name."),
        };
    }
    if host == "pinterest.com" || host.ends_with(".pinterest.com") || host.starts_with("pinterest.") {
        return match segs.as_slice() {
            ["pin", ..] => bail!("This is the link of one pin. Paste the link of the board, or drag the image onto the Board."),
            [user] => Ok(BoardRef::Pinterest { user: user.to_string(), board: None }),
            [user, board, ..] => Ok(BoardRef::Pinterest { user: user.to_string(), board: Some(board.to_string()) }),
            _ => bail!("This is not the link of a Pinterest board. It looks like https://www.pinterest.com/name/board-name/."),
        };
    }
    if host == "pin.it" {
        bail!("Open the short link in a browser first, and paste the full link of the board.");
    }
    bail!("Board import reads Are.na channels and Pinterest boards. \"{host}\" is neither.")
}

/// The client that the board services see. Are.na asks automated clients to
/// name themselves; a contact link is added when the package has a repository.
fn user_agent() -> String {
    let repo = option_env!("CARGO_PKG_REPOSITORY").unwrap_or("");
    format!(
        "Corydoras/{} (desktop app for design references; board import{})",
        env!("CARGO_PKG_VERSION"),
        if repo.is_empty() { String::new() } else { format!("; +{repo}") }
    )
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(Duration::from_secs(40)).user_agent(user_agent()).build()?)
}

/// Waits before the second and the third try of a request, in milliseconds.
const RETRY_WAITS_MS: [u64; 2] = [1_500, 4_000];

/// Answers that say "not now": the service is busy, or a gateway before it timed out.
fn is_busy(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Reads a text answer. A busy service or a broken connection gets two more
/// tries, because one bad page would otherwise fail a whole board.
async fn get_text(c: &reqwest::Client, url: &str, what: &str) -> Result<String> {
    get_text_with(c, url, what, &RETRY_WAITS_MS).await
}

async fn get_text_with(c: &reqwest::Client, url: &str, what: &str, waits_ms: &[u64]) -> Result<String> {
    let mut attempt = 0;
    loop {
        let last = attempt >= waits_ms.len();
        match c.get(url).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status == 404 {
                    bail!("{what} has no such board. Check the link; a private board cannot be imported.");
                }
                if status == 401 || status == 403 {
                    bail!("{what} refused the request (HTTP {status}). The board can be private, or the service blocks automated requests now.");
                }
                if resp.status().is_success() {
                    return Ok(resp.text().await?);
                }
                if !is_busy(status) {
                    bail!("{what} answered HTTP {status}.");
                }
                if last {
                    bail!("{what} is busy (HTTP {status}, {} tries). Import the board again in a minute.", attempt + 1);
                }
            }
            Err(e) => {
                if last {
                    bail!("Cannot reach {what}: {e}");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(waits_ms[attempt])).await;
        attempt += 1;
    }
}

/// The image blocks of one page of an Are.na channel. Returns (items, blocks without an image).
pub fn arena_items(page: &Value) -> (Vec<BoardItem>, usize) {
    let mut items = Vec::new();
    let mut skipped = 0;
    for b in page.get("contents").and_then(|v| v.as_array()).into_iter().flatten() {
        let url = |size: &str| b.pointer(&format!("/image/{size}/url")).and_then(|v| v.as_str()).filter(|u| u.starts_with("http")).map(|u| u.to_string());
        // The large size is at most 1800 px; the original can be very big.
        let Some(image_url) = url("large").or_else(|| url("original")).or_else(|| url("display")) else {
            skipped += 1;
            continue;
        };
        let text = |k: &str| b.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(|s| s.to_string());
        let id = b.get("id").map(|v| v.to_string()).unwrap_or_default();
        items.push(BoardItem {
            fallback_url: url("original").filter(|o| o != &image_url),
            image_url,
            title: text("title").or_else(|| text("generated_title")).unwrap_or_else(|| format!("Are.na block {id}")),
            source: b.pointer("/source/url").and_then(|v| v.as_str()).filter(|u| u.starts_with("http")).map(|u| u.to_string()).unwrap_or_else(|| format!("https://www.are.na/block/{id}")),
        });
    }
    (items, skipped)
}

fn unescape(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).unwrap_or(s);
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&")
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

/// The pins of a Pinterest RSS feed. Returns (board title, items).
pub fn pinterest_items(xml: &str) -> (String, Vec<BoardItem>) {
    let head = xml.split("<item>").next().unwrap_or("");
    let title = between(head, "<title>", "</title>").map(unescape).unwrap_or_default();
    let mut items = Vec::new();
    for raw in xml.split("<item>").skip(1) {
        let raw = raw.split("</item>").next().unwrap_or("");
        let description = between(raw, "<description>", "</description>").map(unescape).unwrap_or_default();
        let Some(src) = between(&description, "src=\"", "\"").filter(|u| u.starts_with("https://")) else { continue };
        // The feed links a small size. The same file exists as the original and as a 736 px size.
        let sized = |size: &str| {
            let mut parts: Vec<&str> = src.split('/').collect();
            match parts.iter().position(|p| p.ends_with('x') && p[..p.len() - 1].chars().all(|c| c.is_ascii_digit()) && p.len() > 1) {
                Some(i) => {
                    parts[i] = size;
                    Some(parts.join("/"))
                }
                None => None,
            }
        };
        let link = between(raw, "<link>", "</link>").map(unescape).unwrap_or_default();
        let name = between(raw, "<title>", "</title>").map(unescape).unwrap_or_default();
        items.push(BoardItem {
            image_url: sized("originals").unwrap_or_else(|| src.to_string()),
            fallback_url: sized("736x").or_else(|| Some(src.to_string())),
            title: if name.trim().is_empty() { "Pinterest pin".to_string() } else { name.trim().to_string() },
            source: if link.starts_with("http") { link } else { src.to_string() },
        });
    }
    (title, items)
}

/// Reads the list of images of a board. Nothing is downloaded yet.
pub async fn fetch_board(url: &str) -> Result<Board> {
    let c = client()?;
    match parse_board_url(url)? {
        BoardRef::Arena { slug } => {
            let mut title = slug.clone();
            if let Ok(text) = get_text(&c, &format!("https://api.are.na/v2/channels/{slug}/thumb"), "Are.na").await {
                if let Some(t) = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("title").and_then(|t| t.as_str()).map(|s| s.to_string())) {
                    title = t;
                }
            }
            let (mut items, mut skipped) = (Vec::new(), 0);
            for page in 1..=20 {
                let text = get_text(&c, &format!("https://api.are.na/v2/channels/{slug}/contents?per={ARENA_PAGE}&page={page}"), "Are.na").await?;
                let v: Value = serde_json::from_str(&text).map_err(|_| anyhow!("Are.na sent an answer that is not JSON."))?;
                let count = v.get("contents").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
                let (mut found, s) = arena_items(&v);
                items.append(&mut found);
                skipped += s;
                if count < ARENA_PAGE || items.len() >= MAX_IMAGES {
                    break;
                }
            }
            let note = (items.len() > MAX_IMAGES).then(|| format!("The channel has more than {MAX_IMAGES} images; the first {MAX_IMAGES} are imported."));
            items.truncate(MAX_IMAGES);
            Ok(Board { service: "Are.na".into(), title, url: url.trim().to_string(), items, skipped, note })
        }
        BoardRef::Pinterest { user, board } => {
            let feed = match &board {
                Some(b) => format!("https://www.pinterest.com/{user}/{b}.rss"),
                None => format!("https://www.pinterest.com/{user}/feed.rss"),
            };
            let xml = get_text(&c, &feed, "Pinterest").await?;
            let (title, items) = pinterest_items(&xml);
            Ok(Board {
                service: "Pinterest".into(),
                title: if title.is_empty() { board.unwrap_or(user) } else { title },
                url: url.trim().to_string(),
                items,
                skipped: 0,
                note: Some("Pinterest gives only the newest 25 pins of a board. For older pins, use the browser extension or drag the images onto the Board.".into()),
            })
        }
    }
}

/// Downloads one image with the client that names the app (the image hosts of
/// the board services can refuse an unnamed client).
async fn download(c: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let resp = c.get(url).send().await.map_err(|e| anyhow!("Download failed: {e}"))?;
    if !resp.status().is_success() {
        bail!("Download failed: HTTP {}", resp.status().as_u16());
    }
    if resp.content_length().map(|l| l > MAX_IMAGE_BYTES).unwrap_or(false) {
        bail!("The image is larger than 80 MB.");
    }
    let bytes = resp.bytes().await?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        bail!("The image is larger than 80 MB.");
    }
    Ok(bytes.to_vec())
}

fn file_name(item: &BoardItem) -> String {
    let ext = item
        .image_url
        .split(['?', '#'])
        .next()
        .and_then(|u| u.rsplit('/').next())
        .and_then(|n| n.rsplit_once('.').map(|(_, e)| e.to_lowercase()))
        .filter(|e| ["jpg", "jpeg", "png", "webp", "gif"].contains(&e.as_str()))
        .unwrap_or_else(|| "jpg".into());
    let stem: String = item.title.chars().map(|c| if c.is_control() || "/\\:".contains(c) { ' ' } else { c }).collect::<String>().trim().chars().take(80).collect();
    format!("{}.{ext}", if stem.is_empty() { "image".to_string() } else { stem })
}

/// Downloads the images of a board and adds them to a language. An image that
/// the language has already is counted, not added again. `progress(done, total)`
/// is called after each image.
pub async fn import_board(lib: &Library, language_id: &str, board: &Board, progress: &(dyn Fn(usize, usize) + Send + Sync)) -> Result<ImportSummary> {
    lib.get_language(language_id)?;
    let c = client()?;
    let mut sum = ImportSummary::default();
    let total = board.items.len();
    for (n, item) in board.items.iter().enumerate() {
        let bytes = match download(&c, &item.image_url).await {
            Ok(b) => Ok(b),
            Err(e) => match &item.fallback_url {
                Some(f) => download(&c, f).await,
                None => Err(e),
            },
        };
        match bytes {
            Ok(b) => {
                if lib.has_item_hash(language_id, &fnv1a64(&b))? {
                    sum.duplicates += 1;
                } else {
                    match lib.import_bytes(language_id, &file_name(item), &b, Some(&item.source)) {
                        Ok(_) => sum.added += 1,
                        Err(e) => sum.errors.push(format!("{}: {e}", item.title)),
                    }
                }
            }
            Err(e) => sum.errors.push(format!("{}: {e}", item.title)),
        }
        progress(n + 1, total);
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn board_links() {
        assert_eq!(parse_board_url("https://www.are.na/charles-broskoski/arena-influences?x=1").unwrap(), BoardRef::Arena { slug: "arena-influences".into() });
        assert_eq!(parse_board_url("https://are.na/channel/soft-forms/").unwrap(), BoardRef::Arena { slug: "soft-forms".into() });
        assert!(parse_board_url("https://www.are.na/block/3235876").is_err());
        assert!(parse_board_url("https://www.are.na/").is_err());
        assert_eq!(parse_board_url("https://www.pinterest.com/someone/quiet-objects/").unwrap(), BoardRef::Pinterest { user: "someone".into(), board: Some("quiet-objects".into()) });
        assert_eq!(parse_board_url("https://de.pinterest.com/someone/").unwrap(), BoardRef::Pinterest { user: "someone".into(), board: None });
        assert!(parse_board_url("https://www.pinterest.com/pin/424605071146306108/").unwrap_err().to_string().contains("one pin"));
        assert!(parse_board_url("https://pin.it/abc").is_err());
        assert!(parse_board_url("https://example.com/board").unwrap_err().to_string().contains("neither"));
        assert!(parse_board_url("are.na/a/b").is_err());
    }

    #[test]
    fn arena_blocks() {
        let page = json!({"contents": [
            {"id": 1, "class": "Image", "title": "Oak bench", "image": {"large": {"url": "https://images.are.na/large-1"}, "original": {"url": "https://d2w9rnfcy7mm78.cloudfront.net/1/original_a.png"}}, "source": null},
            {"id": 2, "class": "Link", "title": "", "generated_title": "A page", "image": {"original": {"url": "https://d2w9rnfcy7mm78.cloudfront.net/2/original_b.jpg"}}, "source": {"url": "https://example.com/page"}},
            {"id": 3, "class": "Text", "content": "A note", "image": null},
            {"id": 4, "class": "Channel"}
        ]});
        let (items, skipped) = arena_items(&page);
        assert_eq!((items.len(), skipped), (2, 2));
        assert_eq!(items[0], BoardItem { image_url: "https://images.are.na/large-1".into(), fallback_url: Some("https://d2w9rnfcy7mm78.cloudfront.net/1/original_a.png".into()), title: "Oak bench".into(), source: "https://www.are.na/block/1".into() });
        assert_eq!((items[1].title.as_str(), items[1].source.as_str(), &items[1].fallback_url), ("A page", "https://example.com/page", &None));
        assert_eq!(file_name(&items[1]), "A page.jpg");
        assert_eq!(file_name(&BoardItem { image_url: "https://x/y.PNG?z=1".into(), fallback_url: None, title: "a/b: c".into(), source: String::new() }), "a b  c.png");
        assert_eq!(arena_items(&json!({})), (vec![], 0));
    }

    #[test]
    fn pinterest_feed() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><rss version="2.0"><channel><title>Quiet objects</title><link>https://www.pinterest.com/someone/quiet-objects/</link>
            <item><title>A lamp &amp; a shade</title><link>https://www.pinterest.com/pin/1/</link>
              <description>&lt;a href=&quot;https://www.pinterest.com/pin/1/&quot;&gt;&lt;img src=&quot;https://i.pinimg.com/236x/e3/9e/69/e39e.jpg&quot;&gt;&lt;/a&gt;A lamp</description></item>
            <item><title> </title><link>https://www.pinterest.com/pin/2/</link><description>no image here</description></item>
            <item><title> </title><link>https://www.pinterest.com/pin/3/</link>
              <description>&lt;img src=&quot;https://i.pinimg.com/236x/aa/bb/cc/aabb.png&quot;&gt;</description></item>
        </channel></rss>"#;
        let (title, items) = pinterest_items(xml);
        assert_eq!(title, "Quiet objects");
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            BoardItem {
                image_url: "https://i.pinimg.com/originals/e3/9e/69/e39e.jpg".into(),
                fallback_url: Some("https://i.pinimg.com/736x/e3/9e/69/e39e.jpg".into()),
                title: "A lamp & a shade".into(),
                source: "https://www.pinterest.com/pin/1/".into()
            }
        );
        assert_eq!(items[1].title, "Pinterest pin");
        assert!(user_agent().starts_with("Corydoras/"));
    }

    /// A server on this computer that answers each request with the next status of the list.
    fn serve(statuses: Vec<u16>) -> (String, std::thread::JoinHandle<usize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut served = 0;
            for status in statuses {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let body = if status == 200 { "ok" } else { "busy" };
                let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                served += 1;
            }
            served
        });
        (format!("http://{addr}/board"), handle)
    }

    #[tokio::test]
    async fn busy_service_gets_more_tries() {
        let c = client().unwrap();
        // Two gateway errors, then the answer.
        let (url, server) = serve(vec![504, 503, 200]);
        assert_eq!(get_text_with(&c, &url, "Test", &[1, 1]).await.unwrap(), "ok");
        assert_eq!(server.join().unwrap(), 3);

        // Busy three times: the error says so, and no fourth request goes out.
        let (url, server) = serve(vec![504, 504, 504]);
        let e = get_text_with(&c, &url, "Test", &[1, 1]).await.unwrap_err().to_string();
        assert!(e.contains("busy") && e.contains("504") && e.contains("3 tries"), "{e}");
        assert_eq!(server.join().unwrap(), 3);

        // A board that does not exist is final at once.
        let (url, server) = serve(vec![404]);
        assert!(get_text_with(&c, &url, "Test", &[1, 1]).await.unwrap_err().to_string().contains("no such board"));
        assert_eq!(server.join().unwrap(), 1);

        // An answer that is no error and not "busy" is final too.
        let (url, server) = serve(vec![400]);
        assert!(get_text_with(&c, &url, "Test", &[1, 1]).await.unwrap_err().to_string().contains("HTTP 400"));
        assert_eq!(server.join().unwrap(), 1);
    }
}
