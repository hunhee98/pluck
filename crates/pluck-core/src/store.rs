//! Resolve where a repo's index lives on disk.
//!
//! Layout: `~/.pluck/<repo-hash>/tantivy/...`. The repo hash is the first
//! 16 hex chars of SHA-256 over the canonical absolute path — stable across
//! runs, collision-resistant, short enough to be readable.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub fn repo_hash(repo_path: &Path) -> Result<String> {
    let canon = std::fs::canonicalize(repo_path)
        .with_context(|| format!("canonicalize {repo_path:?}"))?;
    let mut h = Sha256::new();
    h.update(canon.to_string_lossy().as_bytes());
    let digest = h.finalize();
    let hex = digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(hex[..16].to_string())
}

pub fn pluck_root() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("PLUCK_HOME") {
        return Ok(PathBuf::from(p));
    }
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".pluck"))
}

pub fn repo_index_dir(repo_path: &Path) -> Result<PathBuf> {
    let hash = repo_hash(repo_path)?;
    Ok(pluck_root()?.join(hash))
}

pub fn tantivy_dir(repo_path: &Path) -> Result<PathBuf> {
    Ok(repo_index_dir(repo_path)?.join("tantivy"))
}
