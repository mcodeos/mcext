//! Semantic Tokens result_id and previous content management
//!
//! LSP `textDocument/semanticTokens/full` returns `result_id`, client brings it back in
//! `textDocument/semanticTokens/delta`, we compute delta based on this.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use tower_lsp::lsp_types::{SemanticToken, Url};

/// Tokens state
#[derive(Debug)]
pub struct TokensState {
    next_id: AtomicU64,
    last: RwLock<HashMap<Url, TokenEntry>>,
}

#[derive(Debug)]
struct TokenEntry {
    result_id: String,
    tokens: Vec<SemanticToken>,
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

    /// Store latest tokens for URI (with auto-generated numeric id)
    pub fn store(&self, uri: Url, id: u64, tokens: Vec<SemanticToken>) {
        let result_id = id.to_string();
        let mut last = self.last.write().unwrap_or_else(|e| {
            tracing::warn!("tokens lock poisoned, attempting recovery");
            e.into_inner()
        });
        last.insert(uri, TokenEntry { result_id, tokens });
    }

    /// Store latest tokens for URI with a pre-set string result_id (from RPC)
    pub fn store_with_result_id(&self, uri: Url, result_id: String, tokens: Vec<SemanticToken>) {
        let mut last = self.last.write().unwrap_or_else(|e| {
            tracing::warn!("tokens lock poisoned, attempting recovery");
            e.into_inner()
        });
        last.insert(uri, TokenEntry { result_id, tokens });
    }

    /// Get last stored (result_id, tokens) for URI
    pub fn get(&self, uri: &Url) -> Option<(String, Vec<SemanticToken>)> {
        let last = self.last.read().unwrap_or_else(|e| {
            tracing::warn!("tokens lock poisoned, attempting recovery");
            e.into_inner()
        });
        last.get(uri)
            .map(|e| (e.result_id.clone(), e.tokens.clone()))
    }

    /// §7.6: Get last stored result_id for dedup.
    pub fn last_result_id(&self, uri: &Url) -> Option<String> {
        let last = self.last.read().unwrap_or_else(|e| {
            tracing::warn!("tokens lock poisoned, attempting recovery");
            e.into_inner()
        });
        last.get(uri).map(|e| e.result_id.clone())
    }

    /// Remove URI (cleanup on document close)
    pub fn remove(&self, uri: &Url) {
        let mut last = self.last.write().unwrap_or_else(|e| {
            tracing::warn!("tokens lock poisoned, attempting recovery");
            e.into_inner()
        });
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
        let id_num = state.next_id();
        state.store(uri.clone(), id_num, tokens.clone());
        let (got_id, got_tokens) = state.get(&uri).unwrap();
        assert_eq!(got_id, id_num.to_string());
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
