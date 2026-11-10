//! Lists an Are.na channel or a Pinterest board and, with a library and a
//! language, imports its first images.
//!
//!     cargo run -p corydoras-core --example boardcheck -- <board link> [<library> <language> <count>]
use corydoras_core::{importers, Library};
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(url) = args.first() else {
        eprintln!("usage: boardcheck <board link> [<library> <language> <count>]");
        std::process::exit(2);
    };
    let mut board = importers::fetch_board(url).await?;
    println!("{} · {} · {} images, {} blocks without an image", board.service, board.title, board.items.len(), board.skipped);
    if let Some(n) = &board.note {
        println!("note: {n}");
    }
    for i in board.items.iter().take(3) {
        println!("  {} <- {}", i.title, i.source);
    }
    if let (Some(lib), Some(lang)) = (args.get(1), args.get(2)) {
        let lib = Library::open(&PathBuf::from(lib))?;
        let language = lib.find_language(lang).or_else(|_| lib.create_language(lang, ""))?;
        board.items.truncate(args.get(3).and_then(|n| n.parse().ok()).unwrap_or(3));
        let sum = importers::import_board(&lib, &language.id, &board, &|done, total| println!("  {done}/{total}")).await?;
        println!("added {}, already there {}, errors {:?}", sum.added, sum.duplicates, sum.errors);
    }
    Ok(())
}
