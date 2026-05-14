//! pluck-core
//!
//! Indexing, AST chunking, hybrid (BM25 + semantic) search, and incremental
//! reindex for the pluck daemon and CLI.

pub mod chunker;
pub mod outliner;
pub mod bm25;
pub mod semantic;
pub mod fusion;
pub mod index;
pub mod watcher;
pub mod symbols;

/// Crate version string from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
