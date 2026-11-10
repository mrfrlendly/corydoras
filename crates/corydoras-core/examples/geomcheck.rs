//! Runs the geometry check on a model file and prints the report.
//!
//!     cargo run --release -p corydoras-core --example geomcheck -- <library> <model> [up] [project]
use corydoras_core::geometry::{check_file, report_text, Args};
use corydoras_core::Library;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: geomcheck <library> <model> [up] [project]");
        std::process::exit(2);
    }
    let lib = Library::open(&PathBuf::from(&args[0]))?;
    let report = check_file(
        &lib,
        &PathBuf::from(&args[1]),
        &Args { up: args.get(2).cloned(), project: args.get(3).cloned(), ..Default::default() },
    )?;
    println!("{}", report_text(&report));
    println!("({} s)", report.seconds);
    Ok(())
}
