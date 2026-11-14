//! cargo run -p corydoras-core --example import_workshop_media -- LIBRARY ARCHIVE
use anyhow::{ensure, Result};
use corydoras_core::{inventory_import_media::import_workshop_media, Library};
use std::path::PathBuf;

fn main() -> Result<()> {
    let paths: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    ensure!(
        paths.len() == 2,
        "Usage: import_workshop_media LIBRARY ARCHIVE"
    );
    ensure!(
        paths[0].join("library.sqlite").is_file(),
        "Choose an existing Corydoras library."
    );
    let library = Library::open(&paths[0])?;
    let report = import_workshop_media(&library, &paths[1])?;
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
