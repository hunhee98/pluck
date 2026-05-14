//! Session-scoped state.
//!
//! Tracks every chunk id the server has returned to the agent during the
//! current MCP connection. Phase 2 will use this to emit a one-line
//! `[already-shown @ result N]` placeholder instead of repeating chunk
//! bodies — the lossless savings the daemon design enables and a CLI
//! tool architecturally cannot.

use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct SessionState {
    seen_chunk_ids: HashSet<u64>,
}

impl SessionState {
    pub fn mark_seen(&mut self, chunk_id: u64) {
        self.seen_chunk_ids.insert(chunk_id);
    }

    pub fn was_seen(&self, chunk_id: u64) -> bool {
        self.seen_chunk_ids.contains(&chunk_id)
    }

    pub fn len(&self) -> usize {
        self.seen_chunk_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen_chunk_ids.is_empty()
    }

    pub fn reset(&mut self) {
        self.seen_chunk_ids.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_and_check() {
        let mut s = SessionState::default();
        assert!(s.is_empty());
        s.mark_seen(7);
        s.mark_seen(42);
        assert!(s.was_seen(7));
        assert!(s.was_seen(42));
        assert!(!s.was_seen(100));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn dedup_does_not_double_count() {
        let mut s = SessionState::default();
        s.mark_seen(1);
        s.mark_seen(1);
        s.mark_seen(1);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn reset_clears() {
        let mut s = SessionState::default();
        s.mark_seen(1);
        s.reset();
        assert!(s.is_empty());
    }
}
