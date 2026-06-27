//! Global shared state
//!
//! - [`WorkspaceState`] is the core state of LSP server, held by `Backend` as an `Arc`.
//! - Each open document corresponds to a [`DocumentEntry`], containing Rope buffer and mcc parse results.
//! - Invariant: read/write concurrency safe; mcc calls are serialized via `Analyzer` (see `analyzer` module).

pub mod document;
pub mod scheduler;
pub mod tokens;

pub use document::{apply_changes, DocumentEntry, DocumentVersion};
pub use scheduler::ReparseScheduler;
pub use tokens::TokensState;

use crate::index::IndexWorkerHandle;
use dashmap::DashMap;
use mcc::{McSemSymbolsArcCell, McSemTokensArcCell, McURI};
use ropey::Rope;
use tower_lsp::lsp_types::Url;

/// Global workspace state
pub struct WorkspaceState {
    /// Document storage
    pub documents: DashMap<Url, DocumentEntry>,

    /// Document → semantic token ArcCell (from mcc)
    pub sem_tokens: DashMap<Url, McSemTokensArcCell>,

    /// Document → semantic symbol ArcCell (from mcc)
    pub sem_symbols: DashMap<Url, McSemSymbolsArcCell>,

    /// Registered McURI (from LSP path normalization)
    pub registered_uris: DashMap<Url, McURI>,

    /// Semantic tokens result_id management
    pub tokens: TokensState,

    /// Project-level index worker handle
    pub index: IndexWorkerHandle,

    /// Parse debounce scheduler
    pub scheduler: ReparseScheduler,
}

impl WorkspaceState {
    /// Create a state **without worker thread**
    ///
    /// Suitable for unit tests and library users. Worker only starts in LSP `Backend::new()`.
    pub fn new() -> Self {
        Self {
            documents: DashMap::new(),
            sem_tokens: DashMap::new(),
            sem_symbols: DashMap::new(),
            registered_uris: DashMap::new(),
            tokens: TokensState::new(),
            index: IndexWorkerHandle::inactive(),
            scheduler: ReparseScheduler::new(std::time::Duration::from_millis(150)),
        }
    }

    /// Create a state with worker thread (only used by `Backend::new()`)
    pub fn with_worker() -> Self {
        Self {
            documents: DashMap::new(),
            sem_tokens: DashMap::new(),
            sem_symbols: DashMap::new(),
            registered_uris: DashMap::new(),
            tokens: TokensState::new(),
            index: IndexWorkerHandle::spawn(),
            scheduler: ReparseScheduler::new(std::time::Duration::from_millis(150)),
        }
    }

    /// Read Rope copy of document for URI.
    /// Returns `Some(Rope)` if document exists.
    pub fn document_rope(&self, uri: &Url) -> Option<Rope> {
        self.documents.get(uri).map(|e| e.rope.clone())
    }

    /// Read document version for URI.
    pub fn document_version(&self, uri: &Url) -> Option<DocumentVersion> {
        self.documents.get(uri).map(|e| e.version)
    }

    /// Insert or update document Rope + version
    pub fn insert_document(&self, uri: Url, rope: Rope, version: DocumentVersion) {
        self.documents.insert(uri, DocumentEntry { rope, version });
    }

    /// Remove document
    pub fn remove_document(&self, uri: &Url) {
        self.documents.remove(uri);
        self.sem_tokens.remove(uri);
        self.sem_symbols.remove(uri);
        self.registered_uris.remove(uri);
    }

    /// Write mcc parse result (ArcCell reference)
    pub fn insert_parse(
        &self,
        uri: Url,
        tokens: McSemTokensArcCell,
        symbols: McSemSymbolsArcCell,
        uri_native: McURI,
    ) {
        self.sem_tokens.insert(uri.clone(), tokens);
        self.sem_symbols.insert(uri.clone(), symbols);
        self.registered_uris.insert(uri, uri_native);
    }
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    #[test]
    fn insert_and_read_document() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        let rope = Rope::from_str("component X {}");
        state.insert_document(uri.clone(), rope.clone(), 1);
        assert_eq!(state.document_version(&uri), Some(1));
        let got = state.document_rope(&uri).unwrap();
        assert_eq!(got.len_bytes(), rope.len_bytes());
    }

    #[test]
    fn remove_document_clears_all() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str("x"), 1);
        state.remove_document(&uri);
        assert!(state.document_rope(&uri).is_none());
        assert!(state.document_version(&uri).is_none());
    }

    #[test]
    fn missing_document_returns_none() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///missing.mc").unwrap();
        assert!(state.document_rope(&uri).is_none());
        assert!(state.document_version(&uri).is_none());
    }
}
