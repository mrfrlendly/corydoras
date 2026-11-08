//! End-to-end check of the pipeline against an OpenAI-compatible server.
//!
//!     cargo run -p corydoras-core --example e2e -- <library-dir> <base-url> [model]
//!
//! Set CORYDORAS_API_KEY for the key. It makes a language with three images
//! (one anti-reference, one with a region mark), runs the worker until a
//! version exists, then pins the version in a project.

use corydoras_core::{activity::Activity, agent, models::*, sources, worker, Library};
use std::path::PathBuf;

fn png(r: u8, g: u8, b: u8) -> Vec<u8> {
    let img = image::RgbImage::from_fn(640, 420, |x, y| {
        if (x / 40 + y / 40) % 2 == 0 {
            image::Rgb([r, g, b])
        } else {
            image::Rgb([r / 2, g / 2, b / 2])
        }
    });
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .unwrap();
    buf.into_inner()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let base = args[2].clone();
    let model = args.get(3).cloned().unwrap_or_else(|| "deepseek-flash".into());
    let lib = Library::open(&root)?;
    let act = Activity::default();
    let mut s = lib.settings()?;
    s.base_url = base;
    s.model = model;
    s.debounce_secs = 5;
    lib.save_settings(&s)?;

    let lang = lib.create_language("Quiet Instrument", "Soft matte instruments with one warm accent.")?;
    let a = lib.import_bytes(&lang.id, "warm.png", &png(232, 228, 220), Some("https://example.com/a"))?;
    let b = lib.import_bytes(&lang.id, "orange.png", &png(226, 86, 27), None)?;
    let c = lib.import_bytes(&lang.id, "chrome.png", &png(180, 190, 200), None)?;
    lib.update_item(&a.id, &["body".into(), "edge".into()], "Matte body, soft edges", 5, false)?;
    lib.update_item(&c.id, &["finish".into()], "Glossy chrome", 2, true)?;
    lib.set_regions(&b.id, &[RegionInput { id: None, x: 0.2, y: 0.2, w: 0.3, h: 0.3, note: "accent dot".into(), tag: "accent".into() }])?;
    // Sources: a link (CORYDORAS_E2E_LINK) and a text document.
    if let Ok(url) = std::env::var("CORYDORAS_E2E_LINK") {
        let ex = sources::fetch_link(&url).await?;
        let s = lib.add_link_source(&lang.id, &url, &ex)?;
        println!("link source {} \"{}\" media={} words={} preview={}", &s.id[..8], s.title, s.media, s.word_count, s.preview_rel.is_some());
        lib.update_source(&s.id, &s.title, "Use the fillet rules", 5)?;
        lib.enqueue("analyze_source", &s.id, 0)?;
    }
    if let Ok(path) = std::env::var("CORYDORAS_E2E_DOC") {
        let bytes = std::fs::read(&path)?;
        let name = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().into_owned();
        let s = lib.import_document(&lang.id, &name, &bytes)?;
        println!("document source {} \"{}\" media={} words={}", &s.id[..8], s.title, s.media, s.word_count);
        lib.enqueue("analyze_source", &s.id, 0)?;
    }
    // Run the analysis now, not after the re-analysis delay.
    for i in [&a, &b, &c] {
        lib.enqueue("analyze_item", &i.id, 0)?;
    }

    let mut steps = 0;
    loop {
        steps += 1;
        if steps > 40 {
            anyhow::bail!("No version after 40 steps");
        }
        match worker::step(&lib, &act).await {
            worker::Outcome::Idle => {
                if lib.pending_analyze_count(&lang.id)? == 0 && lib.latest_version(&lang.id)?.is_none() {
                    lib.synthesize_now(&lang.id)?;
                }
                if lib.latest_version(&lang.id)?.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
            worker::Outcome::Did { message, .. } => println!("step {steps}: {message}"),
            worker::Outcome::Paused(m) => anyhow::bail!("Paused: {m}"),
        }
    }
    for i in lib.list_items(&lang.id)? {
        println!("item {} status={} err={:?}", &i.id[..8], i.analysis_status, i.analysis_error);
    }
    let v = lib.read_latest(&lang.id)?.unwrap();
    println!(
        "VERSION {} model={} items={} schema={} warnings={:?}",
        v.info.version, v.info.model, v.info.item_count, v.info.schema, v.info.warnings
    );
    println!("--- language.md (first 900 chars) ---\n{}", &v.markdown.chars().take(900).collect::<String>());

    // A second synthesis with no change must not make a new version.
    lib.synthesize_now(&lang.id)?;
    let _ = worker::step(&lib, &act).await;
    println!("latest after no-change synth: v{}", lib.latest_version(&lang.id)?.unwrap());

    let p = lib.create_project("Desk Lamp")?;
    let rs: Vec<String> = lib.list_rule_sets()?.into_iter().filter(|r| !r.always).map(|r| r.id).take(1).collect();
    lib.set_project_rule_sets(&p.id, &rs)?;
    lib.set_pins(&p.id, &[PinInput { language_id: lang.id.clone(), version: v.info.version, role: "primary".into(), scope: "all".into() }])?;
    let brief = agent::project_brief_text(&lib, "desk lamp")?;
    println!("--- brief (first 300 chars) ---\n{}", &brief.chars().take(300).collect::<String>());
    for x in lib.list_sources(&lang.id)? {
        println!("source {} status={} err={:?}", &x.id[..8], x.analysis_status, x.analysis_error);
    }
    let st = agent::sources_text(&lib, &lang.id, &[])?;
    println!("--- get_sources (first 600 chars) ---\n{}", &st.chars().take(600).collect::<String>());
    println!("usage: {:?}", lib.usage()?);
    println!("--- activity log (oldest first) ---");
    for e in act.snapshot().log.iter().rev() {
        println!("[{}] {}", e.level, e.text);
    }
    Ok(())
}
