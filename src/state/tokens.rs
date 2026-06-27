//! Semantic Tokens result_id and previous content management
//!
//! LSP `textDocument/semanticTokens/full` returns `result_id`, client brings it back in
//! `textDocument/semanticTokens/delta`, we compute delta based on this.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use tower_lsp::lsp_types::{SemanticToken, Url};

/// Tokens state
pub struct TokensState {
    next_id: AtomicU64,
    last: RwLock<HashMap<Url, (u64, Vec<SemanticToken>)>>,
}

impl TokensState {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            last: RwLock::new(HashMap::new()),
        }
    }

    /// Get next result_id (monotonically increasing)
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Store latest tokens for URI
    pub fn store(&self, uri: Url, id: u64, tokens: Vec<SemanticToken>) {
        let mut last = self.last.write().expect("tokens lock poisoned");
        last.insert(uri, (id, tokens));
    }

    /// Get last stored (id, tokens) for URI
    pub fn get(&self, uri: &Url) -> Option<(u64, Vec<SemanticToken>)> {
        let last = self.last.read().expect("tokens lock poisoned");
        last.get(uri).cloned()
    }

    /// Remove URI (cleanup on document close)
    pub fn remove(&self, uri: &Url) {
        let mut last = self.last.write().expect("tokens lock poisoned");
        last.remove(uri);
    }
}

impl Default for TokensState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{Position, Range};

    fn dummy_token(line: u32, start: u32, len: u32, tt: u32) -> SemanticToken {
        SemanticToken {
            delta_line: line,
            delta_start: start,
            length: len,
            token_type: tt,
            token_modifiers_bitset: 0,
        }
    }

    #[test]
    fn next_id_is_monotonic() {
        let state = TokensState::new();
        let a = state.next_id();
        let b = state.next_id();
        let c = state.next_id();
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn store_and_get_roundtrip() {
        let state = TokensState::new();
        let uri = Url::parse("file:///t.mc").unwrap();
        let tokens = vec![dummy_token(0, 0, 3, 13)];
        let id = state.next_id();
        state.store(uri.clone(), id, tokens.clone());
        let (got_id, got_tokens) = state.get(&uri).unwrap();
        assert_eq!(got_id, id);
        assert_eq!(got_tokens.len(), 1);
    }

    #[test]
    fn remove_clears() {
        let state = TokensState::new();
        let uri = Url::parse("file:///t.mc").unwrap();
        state.store(uri.clone(), 1, vec![]);
        state.remove(&uri);
        assert!(state.get(&uri).is_none());
    }

    #[test]
    #[allow(dead_code)]
    fn range_struct_in_scope() {
        // Ensure Range is available in tests (prevent unused import)
        let _r = Range::new(Position::new(0, 0), Position::new(0, 5));
    }
}
