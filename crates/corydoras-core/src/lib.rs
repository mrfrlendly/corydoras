//! Core of Corydoras: the library on disk, the SQLite index, the job queue,
//! the LLM provider, the analysis pipeline and the text that agents read.

pub mod activity;
pub mod ai;
pub mod agent;
pub mod analysis;
pub mod config;
pub mod db;
pub mod library;
pub mod models;
pub mod proposals;
pub mod provider;
pub mod secrets;
pub mod sources;
pub mod spec;
pub mod storage;
pub mod util;
pub mod worker;

pub use library::Library;
