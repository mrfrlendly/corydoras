//! Links and documents that feed the analysis: fetching, text extraction and
//! preview images. No database code here; library.rs stores the results.

use crate::analysis;
use crate::util::truncate;
use anyhow::{anyhow, bail, Result};
use scraper::{Html, Selector};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

/// Most characters of extracted text that the app keeps for one source.
pub const MAX_TEXT_CHARS: usize = 200_000;
/// Largest page or document that the app fetches or imports (bytes).
pub const MAX_BYTES: usize = 80 * 1024 * 1024;

/// Document types that Corydoras can read.
pub const DOC_EXTENSIONS: &[&str] = &["pdf", "txt", "md", "markdown", "csv", "html", "htm", "doc", "docx", "rtf", "odt"];

/// Result of reading a link or a document.
#[derive(Debug, Default, Clone)]
pub struct Extracted {
    pub title: String,
    pub description: String,
    pub site: String,
    /// "page", "video", "pdf", "text" or "doc".
    pub media: String,
    pub text: String,
    /// The URL after redirects (links only).
    pub final_url: String,
    /// JPEG preview, longest side 640 px.
    pub preview: Option<Vec<u8>>,
    /// The fetched file, when a link points at a document (for example a PDF).
    pub file: Option<(String, Vec<u8>)>,
}

fn clean_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_lines = 0;
    for line in s.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() {
            blank_lines += 1;
            if blank_lines == 1 && !out.is_empty() {
                out.push('\n');
            }
        } else {
            blank_lines = 0;
            out.push_str(&l);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

fn cap_text(s: String) -> String {
    if s.chars().count() > MAX_TEXT_CHARS {
        s.chars().take(MAX_TEXT_CHARS).collect()
    } else {
        s
    }
}

pub fn word_count(s: &str) -> i64 {
    s.split_whitespace().count() as i64
}

fn preview_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = analysis::decode_oriented(bytes).ok()?;
    analysis::encode_jpeg(&img, 640, 82).ok()
}

// ----------------------------------------------------------------------
// HTML
// ----------------------------------------------------------------------

fn meta(doc: &Html, keys: &[&str]) -> String {
    for k in keys {
        for attr in ["property", "name"] {
            if let Ok(sel) = Selector::parse(&format!("meta[{attr}=\"{k}\"]")) {
                if let Some(el) = doc.select(&sel).next() {
                    if let Some(c) = el.value().attr("content") {
                        let c = c.trim();
                        if !c.is_empty() {
                            return c.to_string();
                        }
                    }
                }
            }
        }
    }
    String::new()
}

/// Readable text of an HTML page: the main or article element when there is
/// one, without scripts, styles, navigation, headers and footers.
pub fn html_to_text(html: &str) -> String {
    let doc = Html::parse_document(html);
    let skip = ["script", "style", "noscript", "nav", "header", "footer", "aside", "form", "svg", "button", "iframe"];
    let root_sel = ["article", "main", "[role=main]", "body"];
    let mut root = None;
    for r in root_sel {
        if let Ok(sel) = Selector::parse(r) {
            if let Some(el) = doc.select(&sel).next() {
                root = Some(el);
                break;
            }
        }
    }
    let Some(root) = root else { return String::new() };
    let block = ["p", "div", "li", "h1", "h2", "h3", "h4", "h5", "h6", "tr", "br", "section", "pre", "blockquote", "td", "dd", "dt"];
    let mut out = String::new();
    fn walk(node: ego_tree::NodeRef<'_, scraper::Node>, out: &mut String, skip: &[&str], block: &[&str]) {
        for child in node.children() {
            match child.value() {
                scraper::Node::Text(t) => out.push_str(t),
                scraper::Node::Element(e) => {
                    let name = e.name();
                    if skip.contains(&name) {
                        continue;
                    }
                    if name.starts_with('h') && name.len() == 2 {
                        out.push_str("\n\n## ");
                    } else if name == "li" {
                        out.push_str("\n- ");
                    }
                    walk(child, out, skip, block);
                    if block.contains(&name) {
                        out.push('\n');
                    }
                }
                _ => {}
            }
        }
    }
    walk(*root, &mut out, &skip, &block);
    clean_ws(&out)
}

fn html_title(doc: &Html) -> String {
    let t = meta(doc, &["og:title", "twitter:title"]);
    if !t.is_empty() {
        return t;
    }
    Selector::parse("title")
        .ok()
        .and_then(|s| doc.select(&s).next().map(|e| e.text().collect::<String>().trim().to_string()))
        .unwrap_or_default()
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_string()
}

fn absolute(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https://{rest}");
    }
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let origin_end = base[scheme_end..].find('/').map(|i| i + scheme_end).unwrap_or(base.len());
    if href.starts_with('/') {
        format!("{}{}", &base[..origin_end], href)
    } else {
        let dir_end = base.rfind('/').filter(|i| *i >= origin_end).unwrap_or(origin_end);
        format!("{}/{}", &base[..dir_end], href)
    }
}

pub fn is_youtube(url: &str) -> bool {
    let h = host_of(url);
    h == "youtube.com" || h == "m.youtube.com" || h == "youtu.be" || h == "music.youtube.com"
}

// ----------------------------------------------------------------------
// Links
// ----------------------------------------------------------------------

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15")
        .build()?)
}

async fn get_bytes(c: &reqwest::Client, url: &str) -> Result<(Vec<u8>, String, String)> {
    let resp = c.get(url).send().await.map_err(|e| {
        if e.is_timeout() {
            anyhow!("{url} did not answer in time. Try again later.")
        } else if e.is_connect() {
            anyhow!("Cannot connect to {}. Check the link and your network.", host_of(url))
        } else {
            anyhow!("Cannot open {url}: {e}")
        }
    })?;
    if !resp.status().is_success() {
        bail!("The page answered HTTP {}.", resp.status().as_u16());
    }
    if resp.content_length().map(|l| l as usize > MAX_BYTES).unwrap_or(false) {
        bail!("The file is larger than 80 MB.");
    }
    let final_url = resp.url().to_string();
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let bytes = resp.bytes().await?.to_vec();
    if bytes.len() > MAX_BYTES {
        bail!("The file is larger than 80 MB.");
    }
    Ok((bytes, ctype, final_url))
}

/// Fetches a link and reads its title, description, preview image and text.
pub async fn fetch_link(url: &str) -> Result<Extracted> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("A link must start with http:// or https://.");
    }
    let c = client()?;
    let (bytes, ctype, final_url) = get_bytes(&c, url).await?;
    let name_from_url = final_url
        .split(['?', '#'])
        .next()
        .and_then(|u| u.rsplit('/').next())
        .filter(|s| !s.is_empty())
        .unwrap_or("document")
        .to_string();

    // A link straight to a document.
    let looks_pdf = ctype.contains("application/pdf") || bytes.starts_with(b"%PDF");
    if looks_pdf || ctype.starts_with("text/plain") || ctype.contains("markdown") {
        let name = if looks_pdf && !name_from_url.to_lowercase().ends_with(".pdf") {
            format!("{name_from_url}.pdf")
        } else {
            name_from_url.clone()
        };
        let mut ex = extract_document(&name, &bytes, None)?;
        ex.site = host_of(&final_url);
        ex.final_url = final_url;
        ex.file = Some((name, bytes));
        return Ok(ex);
    }
    if ctype.starts_with("image/") {
        bail!("This link is an image. Add it on the Board instead.");
    }

    let html = String::from_utf8_lossy(&bytes).into_owned();
    let (title, description, site, image_url, mut text) = {
        let doc = Html::parse_document(&html);
        let title = html_title(&doc);
        let description = meta(&doc, &["og:description", "description", "twitter:description"]);
        let site = {
            let s = meta(&doc, &["og:site_name", "application-name"]);
            if s.is_empty() { host_of(&final_url) } else { s }
        };
        let image = meta(&doc, &["og:image", "og:image:url", "twitter:image", "twitter:image:src"]);
        (title, description, site, image, html_to_text(&html))
    };
    let mut ex = Extracted {
        title: if title.is_empty() { host_of(&final_url) } else { truncate(&title, 200) },
        description: truncate(&description, 600),
        site,
        media: "page".into(),
        final_url: final_url.clone(),
        ..Default::default()
    };
    let mut image_url = if image_url.is_empty() { None } else { Some(absolute(&final_url, &image_url)) };

    if is_youtube(&final_url) || is_youtube(url) {
        ex.media = "video".into();
        // oEmbed gives a reliable title, channel and thumbnail.
        let o = format!("https://www.youtube.com/oembed?format=json&url={}", urlencode(url));
        if let Ok((ob, _, _)) = get_bytes(&c, &o).await {
            if let Ok(v) = serde_json::from_slice::<Value>(&ob) {
                if let Some(t) = v["title"].as_str() {
                    ex.title = t.to_string();
                }
                if let Some(a) = v["author_name"].as_str() {
                    ex.site = format!("YouTube · {a}");
                }
                if let Some(th) = v["thumbnail_url"].as_str() {
                    image_url = Some(th.to_string());
                }
            }
        }
        // The page text of a video site is mostly menus; keep what describes the video.
        text = format!("Video title: {}\nChannel: {}\nDescription:\n{}", ex.title, ex.site, ex.description);
    }
    ex.text = cap_text(text);
    if let Some(iu) = image_url {
        if let Ok((ib, _, _)) = get_bytes(&c, &iu).await {
            ex.preview = preview_jpeg(&ib);
        }
    }
    Ok(ex)
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ----------------------------------------------------------------------
// Documents
// ----------------------------------------------------------------------

pub fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn is_document_name(name: &str) -> bool {
    DOC_EXTENSIONS.contains(&extension(name).as_str())
}

fn pdf_text(bytes: &[u8]) -> Result<String> {
    // pdf-extract can panic on unusual files; keep the app alive.
    let owned = bytes.to_vec();
    match std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&owned)) {
        Ok(Ok(t)) => Ok(t),
        Ok(Err(e)) => Err(anyhow!("Cannot read the PDF text: {e}")),
        Err(_) => Err(anyhow!("Cannot read the PDF text (the file has an unusual structure).")),
    }
}

/// Pages of a PDF that a vision model reads when the text is missing or thin.
pub const PDF_IMAGE_PAGES: usize = 8;
/// Longest side, in pixels, of a rendered PDF page.
pub const PDF_PAGE_SIDE: u32 = 1400;
/// A PDF with fewer text characters per page than this is read from page images.
pub const PDF_SPARSE_CHARS_PER_PAGE: i64 = 300;

/// Renders the first `max_pages` pages of a PDF as JPEG images. Returns the
/// images and the number of pages in the file.
pub fn pdf_pages(bytes: &[u8], max_pages: usize, max_side: u32) -> Result<(Vec<Vec<u8>>, usize)> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{render, PixmapSettings, RenderCache, RenderSettings};

    let owned = bytes.to_vec();
    // The renderer can panic on unusual files; keep the app alive.
    let result = std::panic::catch_unwind(move || -> Result<(Vec<Vec<u8>>, usize)> {
        let pdf = Pdf::new(owned).map_err(|e| anyhow!("Cannot open the PDF: {e:?}"))?;
        let pages = pdf.pages();
        let total = pages.len();
        let cache = RenderCache::new();
        let interp = InterpreterSettings::default();
        let rs = RenderSettings::default();
        let mut out = Vec::new();
        for page in pages.iter().take(max_pages) {
            let (w, h) = page.render_dimensions();
            let scale = (max_side as f32 / w.max(h).max(1.0)).clamp(0.1, 4.0);
            let ps = PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE };
            let png = render(page, &cache, &interp, &rs, &ps)
                .into_png()
                .map_err(|e| anyhow!("Cannot encode a PDF page: {e:?}"))?;
            let img = crate::analysis::decode_oriented(&png)?;
            out.push(crate::analysis::encode_jpeg(&img, max_side, 85)?);
        }
        Ok((out, total))
    });
    match result {
        Ok(r) => r,
        Err(_) => Err(anyhow!("Cannot draw the PDF pages (the file has an unusual structure).")),
    }
}

/// macOS: converts doc, docx, rtf and odt to text with textutil.
#[cfg(target_os = "macos")]
fn textutil(name: &str, bytes: &[u8]) -> Result<String> {
    let dir = std::env::temp_dir().join(format!("corydoras-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(Path::new(name).file_name().unwrap_or_default());
    std::fs::write(&path, bytes)?;
    let out = std::process::Command::new("/usr/bin/textutil")
        .args(["-convert", "txt", "-stdout"])
        .arg(&path)
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    let out = out.map_err(|e| anyhow!("textutil failed: {e}"))?;
    if !out.status.success() {
        bail!("macOS could not read \"{name}\".");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(not(target_os = "macos"))]
fn textutil(name: &str, _bytes: &[u8]) -> Result<String> {
    bail!("\"{name}\": this document type needs macOS (textutil).")
}

/// macOS: a Quick Look thumbnail of the first page.
#[cfg(target_os = "macos")]
pub fn quicklook_preview(path: &Path) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("corydoras-ql-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok()?;
    let status = std::process::Command::new("/usr/bin/qlmanage")
        .args(["-t", "-s", "640", "-o"])
        .arg(&dir)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let png = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "png").unwrap_or(false));
    let out = if status.success() { png.and_then(|p| std::fs::read(p).ok()).and_then(|b| preview_jpeg(&b)) } else { None };
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[cfg(not(target_os = "macos"))]
pub fn quicklook_preview(_path: &Path) -> Option<Vec<u8>> {
    None
}

/// The first line of a document when it looks like a heading: short, with no end stop.
fn doc_title(text: &str) -> Option<String> {
    let first = text.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty())?;
    let words = first.split_whitespace().count();
    let ok = (2..=14).contains(&words) && first.chars().count() <= 100 && !first.ends_with(['.', ',', ';', ':']);
    ok.then(|| first.to_string())
}

/// Reads the text of a document. `saved` is the stored copy, used for the preview.
pub fn extract_document(name: &str, bytes: &[u8], saved: Option<&Path>) -> Result<Extracted> {
    if bytes.is_empty() {
        bail!("\"{name}\" is empty.");
    }
    if bytes.len() > MAX_BYTES {
        bail!("\"{name}\" is larger than 80 MB.");
    }
    let ext = extension(name);
    let is_pdf = ext == "pdf" || bytes.starts_with(b"%PDF");
    let (media, text) = match ext.as_str() {
        "pdf" => ("pdf", pdf_text(bytes).unwrap_or_default()),
        "txt" | "md" | "markdown" | "csv" => ("text", String::from_utf8_lossy(bytes).into_owned()),
        "html" | "htm" => ("text", html_to_text(&String::from_utf8_lossy(bytes))),
        "doc" | "docx" | "rtf" | "odt" => ("doc", textutil(name, bytes)?),
        _ if is_pdf => ("pdf", pdf_text(bytes).unwrap_or_default()),
        _ => bail!("\"{name}\" is not a document type that Corydoras can read ({}).", DOC_EXTENSIONS.join(", ")),
    };
    let text = cap_text(clean_ws(&text));
    // A scanned PDF has no text. The model reads its page images instead.
    let mut first_page = None;
    if text.trim().len() < 20 {
        if !is_pdf {
            bail!("\"{name}\" has no readable text.");
        }
        match pdf_pages(bytes, 1, 640) {
            Ok((mut pages, _)) if !pages.is_empty() => first_page = pages.pop(),
            Ok(_) => bail!("\"{name}\" has no pages."),
            Err(e) => bail!("\"{name}\" has no readable text, and its pages cannot be drawn: {e}"),
        }
    }
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().replace(['_', '-'], " "))
        .unwrap_or_else(|| name.to_string());
    let title = doc_title(&text).unwrap_or(stem);
    let description = truncate(
        &text
            .lines()
            .map(str::trim)
            .find(|l| l.split_whitespace().count() > 6 && *l != title)
            .unwrap_or("")
            .to_string(),
        300,
    );
    Ok(Extracted {
        title,
        description,
        site: String::new(),
        media: media.into(),
        text,
        final_url: String::new(),
        preview: saved.and_then(quicklook_preview).or(first_page),
        file: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_text_and_meta() {
        let html = r#"<html><head><title>Fillets 101</title><meta property="og:description" content="How to size fillets."><meta property="og:image" content="/img/a.jpg"></head>
            <body><nav>Menu Home About</nav><article><h1>Fillets</h1><p>Use a radius of  0.4 times the wall.</p><script>var x=1;</script><ul><li>One</li><li>Two</li></ul></article><footer>Copyright</footer></body></html>"#;
        let t = html_to_text(html);
        assert!(t.contains("## Fillets"));
        assert!(t.contains("Use a radius of 0.4 times the wall."));
        assert!(t.contains("- One"));
        assert!(!t.contains("Menu") && !t.contains("var x") && !t.contains("Copyright"));
        let doc = Html::parse_document(html);
        assert_eq!(html_title(&doc), "Fillets 101");
        assert_eq!(meta(&doc, &["og:description"]), "How to size fillets.");
        assert_eq!(absolute("https://ex.com/blog/post", "/img/a.jpg"), "https://ex.com/img/a.jpg");
        assert_eq!(absolute("https://ex.com/blog/post", "a.jpg"), "https://ex.com/blog/a.jpg");
    }

    #[test]
    fn documents() {
        let ex = extract_document("design_notes.md", b"# Notes\n\nKeep every edge soft and every wall even in thickness.", None).unwrap();
        assert_eq!(ex.media, "text");
        assert_eq!(ex.title, "design notes");
        let ex = extract_document("x.txt", b"Fillets for printed housings\n\nUse a minimum outside fillet of 0.8 mm on PETG parts.", None).unwrap();
        assert_eq!(ex.title, "Fillets for printed housings");
        assert!(extract_document("a.txt", b"short", None).is_err());
        assert!(extract_document("a.xyz", b"some text here that is long enough", None).is_err());
        assert!(is_youtube("https://youtu.be/abc") && is_youtube("https://www.youtube.com/watch?v=x"));
        assert!(!is_youtube("https://example.com/youtube"));
    }

    /// A one-page PDF with a drawn rectangle and no text, like a scan.
    fn blank_pdf() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R /Resources << >> >>".to_string(),
            {
                let c = "0 0 0 rg 20 20 160 60 re f";
                format!("<< /Length {} >>\nstream\n{c}\nendstream", c.len())
            },
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for off in offsets {
            out.extend(format!("{off:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
        out
    }

    #[test]
    fn scanned_pdf_is_accepted_and_drawn() {
        let pdf = blank_pdf();
        let (pages, total) = pdf_pages(&pdf, 8, 400).unwrap();
        assert_eq!((pages.len(), total), (1, 1));
        assert!(pages[0].starts_with(&[0xFF, 0xD8]), "a JPEG");
        let ex = extract_document("scan.pdf", &pdf, None).unwrap();
        assert_eq!(ex.media, "pdf");
        assert!(ex.text.is_empty());
        assert!(ex.preview.is_some());
        assert_eq!(ex.title, "scan");
        // Broken PDF bytes give an error and do not panic.
        assert!(pdf_pages(b"%PDF-1.4 not a pdf", 8, 400).is_err());
        assert!(extract_document("bad.pdf", b"%PDF-1.4 not a pdf", None).is_err());
    }
}
