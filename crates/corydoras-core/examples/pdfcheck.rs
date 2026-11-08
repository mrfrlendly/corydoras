//! Reads each PDF given on the command line the way the app does and reports the result.
use corydoras_core::sources;
fn main() {
    for p in std::env::args().skip(1) {
        let bytes = std::fs::read(&p).unwrap();
        let name = std::path::Path::new(&p).file_name().unwrap().to_string_lossy().into_owned();
        let t = std::time::Instant::now();
        match sources::extract_document(&name, &bytes, Some(std::path::Path::new(&p))) {
            Ok(e) => println!("OK  {name}: title={:?} chars={} preview={} ({:?})", e.title, e.text.len(), e.preview.is_some(), t.elapsed()),
            Err(e) => println!("ERR {name}: {e}"),
        }
        match sources::pdf_pages(&bytes, 3, 1400) {
            Ok((pages, total)) => {
                for (i, p) in pages.iter().enumerate() {
                    if let Ok(dir) = std::env::var("PDF_OUT") {
                        std::fs::write(format!("{dir}/{name}-{}.jpg", i + 1), p).unwrap();
                    }
                }
                println!("    pages: {} of {total} drawn, {} bytes", pages.len(), pages.iter().map(|p| p.len()).sum::<usize>());
            }
            Err(e) => println!("    pages: ERR {e}"),
        }
    }
}
