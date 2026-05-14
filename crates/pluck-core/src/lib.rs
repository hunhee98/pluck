//! pluck-core
//!
//! Indexing, AST chunking, hybrid (BM25 + semantic) search, and incremental
//! reindex for the pluck daemon and CLI.

pub mod bm25;
pub mod callees;
pub mod chunker;
pub mod fusion;
pub mod index;
pub mod indexer;
pub mod outliner;
pub mod semantic;
pub mod store;
pub mod tokenizer;
pub mod symbols;
pub mod watcher;

/// Crate version string from Cargo.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
