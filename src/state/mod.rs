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
use crate::rpc::{CrossFileTarget, LapperEntry, LocalReference, SemSymbols, SymbolEntry};
use dashmap::DashMap;
use ropey::Rope;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::{Mutex as TokioMutex, Notify};
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

impl From<SemSymbols> for RpcSemSymbols {
    fn from(sem: SemSymbols) -> Self {
        Self {
            lapper: sem.lapper,
            local_declares: sem
                .local
                .declares
                .into_iter()
                .map(|d| LocalDeclareSpan {
                    id: d.id,
                    span: d.span,
                })
                .collect(),
            local_references: sem.local.references,
            global_declares: sem
                .global
                .declares
                .into_iter()
                .map(|d| GlobalDeclareSpan {
                    id: d.id,
                    uri: d.uri,
                    span: d.span,
                })
                .collect(),
            global_references: sem
                .global
                .references
                .into_iter()
                .map(|r| GlobalReferenceSpan {
                    id: r.id,
                    uri: r.uri,
                    span: r.span,
                })
                .collect(),
            cross_file_targets: sem.global.cross_file_targets,
        }
    }
}

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

    /// URIs whose initial parse_and_publish failed (e.g. mcc not yet connected).
    /// Retried once the server is ready. Key is the URI; value is the document version at open time.
    pub pending_diagnostics: DashMap<Url, Option<i32>>,

    /// Sticky flag: true after project initialization completes.
    /// Used for fast-path check (no wait needed after init is done).
    pub init_done: AtomicBool,

    /// Wakeup signal for tasks waiting on initialization.
    /// NOT sticky — only used to wake sleepers; always check init_done after waking.
    pub init_notify: Notify,

    /// Serializes RPC calls to mcc (single-threaded server).
    /// Prevents concurrent sem/diagnostics requests from crashing mcc.
    pub rpc_lock: TokioMutex<()>,
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
            pending_diagnostics: DashMap::new(),
            init_done: AtomicBool::new(false),
            init_notify: Notify::new(),
            rpc_lock: TokioMutex::new(()),
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
            pending_diagnostics: DashMap::new(),
            init_done: AtomicBool::new(false),
            init_notify: Notify::new(),
            rpc_lock: TokioMutex::new(()),
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

/// Adjust cached lapper entry offsets after incremental text changes so
/// F12 goto-def stays accurate before the debounced reparse completes.
///
/// For each change (delete [old_start, old_end), insert `new_text`):
///   - byte delta = new_text.len() - (old_end - old_start)
///   - entries with start >= old_end are shifted by delta
///   - entries inside the replaced range are dropped
pub fn adjust_lapper_for_changes(
    state: &WorkspaceState,
    uri: &Url,
    changes: &[tower_lsp::lsp_types::TextDocumentContentChangeEvent],
    rope: &ropey::Rope,
) {
    use crate::common::position::position_to_offset;
    use crate::rpc::LapperEntry;

    let symbols_ref = match state.sem_symbols.get(uri) {
        Some(s) => s,
        None => return,
    };
    let mut symbols = match symbols_ref.lock() {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut entries: Vec<LapperEntry> = std::mem::take(&mut symbols.lapper);

    for change in changes {
        let range = match change.range {
            Some(r) => r,
            None => {
                // Full replace — drop all entries; fresh data arrives via reparse
                return;
            }
        };

        let old_start = match position_to_offset(range.start, rope) {
            Some(o) => o,
            None => continue,
        };
        let old_end = match position_to_offset(range.end, rope) {
            Some(o) => o,
            None => continue,
        };

        let delta: i64 = change.text.len() as i64 - (old_end as i64 - old_start as i64);

        entries = entries
            .into_iter()
            .filter_map(|mut e| {
                if e.start >= old_end {
                    // After the change: shift
                    e.start = (e.start as i64 + delta).max(0) as usize;
                    e.stop = (e.stop as i64 + delta).max(0) as usize;
                    Some(e)
                } else if e.start >= old_start {
                    // Inside the replaced range: drop (stale)
                    None
                } else {
                    // Before the change: unchanged
                    Some(e)
                }
            })
            .collect();
    }

    symbols.lapper = entries;
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
