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
use crate::rpc::{CrossFileTarget, LapperEntry, LocalReference, SymbolEntry};
use dashmap::DashMap;
use ropey::Rope;
use std::sync::{Arc, Mutex};
use tower_lsp::lsp_types::Url;

/// ★ Cached project-wide symbols for completion
#[derive(Debug, Clone, Default)]
pub struct ProjectSymbolsCache {
    pub components: Vec<SymbolEntry>,
    pub interfaces: Vec<SymbolEntry>,
    pub enums: Vec<SymbolEntry>,
    pub modules: Vec<SymbolEntry>,
    /// ★ Per-value rows of every `enum Foo { BAR, BAZ }` declared in the
    ///   project. Used by F12 on `Foo.BAR` in usage sites.
    #[allow(dead_code)]
    pub enum_values: Vec<crate::rpc::EnumValueEntry>,
}

/// ★ RPC-based semantic tokens (replaces McSemTokensArcCell)
#[derive(Debug, Clone, Default)]
pub struct RpcSemTokens {
    pub tokens: Vec<SemTokenEntry>,
}

#[derive(Debug, Clone)]
pub struct SemTokenEntry {
    pub type_: i16,
    pub position: i32,
    pub length: i32,
}

pub type RpcSemTokensArcCell = Arc<Mutex<RpcSemTokens>>;

/// ★ RPC-based semantic symbols (replaces McSemSymbolsArcCell)
/// This structure holds data received from mcc via RPC, not direct mcc library types.
#[derive(Debug, Clone, Default)]
pub struct RpcSemSymbols {
    /// Token type classification intervals (for semantic highlighting)
    pub lapper: Vec<LapperEntry>,
    /// Local declarations: span info for definitions in this file
    pub local_declares: Vec<LocalDeclareSpan>,
    /// Local references: span info for usages in this file
    pub local_references: Vec<LocalReference>,
    /// Global declarations from this file
    pub global_declares: Vec<GlobalDeclareSpan>,
    /// Global references from this file  
    pub global_references: Vec<GlobalReferenceSpan>,
    /// Cross-file goto targets: ref_id -> (target_uri, span)
    pub cross_file_targets: Vec<CrossFileTarget>,
}

#[derive(Debug, Clone)]
pub struct LocalDeclareSpan {
    pub id: u32,
    pub span: [usize; 2],
}

#[derive(Debug, Clone)]
pub struct GlobalDeclareSpan {
    pub id: u32,
    pub uri: String,
    pub span: [usize; 2],
}

#[derive(Debug, Clone)]
pub struct GlobalReferenceSpan {
    pub id: u32,
    pub uri: String,
    pub span: [usize; 2],
}

pub type RpcSemSymbolsArcCell = Arc<Mutex<RpcSemSymbols>>;

/// Global workspace state
pub struct WorkspaceState {
    /// Document storage
    pub documents: DashMap<Url, DocumentEntry>,

    /// Document → RPC semantic tokens (replaces McSemTokensArcCell)
    pub sem_tokens: DashMap<Url, RpcSemTokensArcCell>,

    /// Document → RPC semantic symbols (replaces McSemSymbolsArcCell)
    pub sem_symbols: DashMap<Url, RpcSemSymbolsArcCell>,

    /// Registered McURI strings (from LSP path normalization)
    pub registered_uris: DashMap<Url, String>,

    /// Semantic tokens result_id management
    pub tokens: TokensState,

    /// Project-level index worker handle
    pub index: IndexWorkerHandle,

    /// Parse debounce scheduler
    pub scheduler: ReparseScheduler,

    /// ★ Cached project-wide symbols for completion
    pub project_symbols: Arc<Mutex<ProjectSymbolsCache>>,
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
            project_symbols: Arc::new(Mutex::new(ProjectSymbolsCache::default())),
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
            project_symbols: Arc::new(Mutex::new(ProjectSymbolsCache::default())),
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
