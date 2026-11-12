//! Core of Corydoras: the library on disk, the SQLite index, the job queue,
//! the LLM provider, the analysis pipeline and the text that agents read.

pub mod activity;
pub mod ai;
pub mod agent;
pub mod analysis;
pub mod config;
pub mod conformance;
pub mod cost;
pub mod coverage;
pub mod db;
pub mod diff;
pub mod export;
pub mod geometry;
pub mod importers;
pub mod inherit;
pub mod insight;
pub mod library;
pub mod mesh;
pub mod models;
pub mod outputs;
pub mod palette;
pub mod printlog;
pub mod proposals;
pub mod provider;
pub mod render;
pub mod secrets;
pub mod signin;
pub mod slicer;
pub mod snapshot;
pub mod sources;
pub mod spec;
pub mod storage;
#[cfg(test)]
pub(crate) mod testserver;
pub mod util;
pub mod video;
pub mod worker;

pub use library::Library;
