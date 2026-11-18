//! Live acceptance run for the multi-item label reader. It SPENDS TOKENS on the analysis provider
//! that is saved in the app settings, so run it only on photos that the designer chose.
//!
//!     cargo run -p corydoras-core --example read_label_photos -- OUTPUT.json PHOTO [PHOTO...]
//!
//! It reads each photo in a scratch library, so the designer's library is never opened or changed.
//! The JSON report lists every draft with its specs and sources. A draft with a quantity on hand,
//! or a photo that fails, is flagged, because the reader must never set stock.
use anyhow::{ensure, Result};
use corydoras_core::{ai::AiConfig, config::AppConfig, inventory_label, Library};
use serde_json::{json, Value};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        args.len() >= 2,
        "Usage: read_label_photos OUTPUT.json PHOTO [PHOTO...]"
    );
    let scratch = tempfile::tempdir()?;
    let library = Library::open(scratch.path())?;
    let cfg = AiConfig::effective(AppConfig::load().ai.as_ref(), &library.settings()?);
    let mut report = Vec::new();
    for photo in &args[1..] {
        let name = photo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let entry = match inventory_label::read_labels(&library, &cfg, &std::fs::read(photo)?).await
        {
            Ok(drafts) => {
                let stock_set = drafts.iter().any(|d| d.quantity.is_some());
                println!(
                    "{name}: {} items{}",
                    drafts.len(),
                    if stock_set {
                        "  FLAG: a quantity on hand was set"
                    } else {
                        ""
                    }
                );
                for d in &drafts {
                    println!("  {} | {}", d.name, d.dimensions);
                }
                json!({"photo": name, "count": drafts.len(), "stock_set": stock_set, "drafts": drafts})
            }
            Err(e) => {
                println!("{name}: FAILED {e}");
                json!({"photo": name, "error": e.to_string()})
            }
        };
        report.push(entry);
    }
    let usage = library.usage()?;
    println!(
        "Tokens spent: {} in, {} out, {} requests",
        usage.input_tokens, usage.output_tokens, usage.requests
    );
    let out: Value = json!({"photos": report, "input_tokens": usage.input_tokens, "output_tokens": usage.output_tokens});
    std::fs::write(&args[0], serde_json::to_string_pretty(&out)?)?;
    Ok(())
}
