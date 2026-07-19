//! Global shared state — split into focused sub-structs for testability.
//!
//! - [`WorkspaceState`] is the core state of the LSP server, held by `Backend` as an `Arc`.
//! - Each sub-struct groups related fields and can be borrowed independently.

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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{Mutex as TokioMutex, Notify};
use tower_lsp::lsp_types::Url;

// ============================================================================
// Data types
// ============================================================================

/// Cached project-wide symbols for completion.
#[derive(Debug, Clone, Default)]
pub struct ProjectSymbolsCache {
    pub components: Vec<SymbolEntry>,
    pub interfaces: Vec<SymbolEntry>,
    pub enums: Vec<SymbolEntry>,
    pub modules: Vec<SymbolEntry>,
    /// Per-value rows of every `enum Foo { BAR, BAZ }` declared in the project.
    /// Used by F12 on `Foo.BAR` in usage sites.
    #[allow(dead_code)]
    pub enum_values: Vec<crate::rpc::EnumValueEntry>,
}

/// RPC-based semantic tokens.
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

/// RPC-based semantic symbols — data received from mcc via RPC.
#[derive(Debug, Clone, Default)]
pub struct RpcSemSymbols {
    pub lapper: Vec<LapperEntry>,
    pub local_declares: Vec<LocalDeclareSpan>,
    pub local_references: Vec<LocalReference>,
    pub global_declares: Vec<GlobalDeclareSpan>,
    pub global_references: Vec<GlobalReferenceSpan>,
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
                .map(|d| LocalDeclareSpan { id: d.id, span: d.span })
                .collect(),
            local_references: sem.local.references,
            global_declares: sem
                .global
                .declares
                .into_iter()
                .map(|d| GlobalDeclareSpan { id: d.id, uri: d.uri, span: d.span })
                .collect(),
            global_references: sem
                .global
                .references
                .into_iter()
                .map(|r| GlobalReferenceSpan { id: r.id, uri: r.uri, span: r.span })
                .collect(),
            cross_file_targets: sem.global.cross_file_targets,
        }
    }
}

// ============================================================================
// Sub-structs
// ============================================================================

/// Document storage: text buffers + LSP URI → mcc URI mapping.
#[derive(Debug)]
pub struct DocumentStore {
    pub documents: DashMap<Url, DocumentEntry>,
    pub registered_uris: DashMap<Url, String>,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self {
            documents: DashMap::new(),
            registered_uris: DashMap::new(),
        }
    }

    /// Read a clone of the Rope for `uri`.
    pub fn rope(&self, uri: &Url) -> Option<Rope> {
        self.documents.get(uri).map(|e| e.rope.clone())
    }

    /// Get the document version for `uri`.
    pub fn version(&self, uri: &Url) -> Option<i32> {
        self.documents.get(uri).map(|e| e.version)
    }

    /// Insert or update a document. Skips if `version <= current` and `version >= 0`.
    pub fn insert(&self, uri: Url, rope: Rope, version: i32) {
        use dashmap::mapref::entry::Entry;
        match self.documents.entry(uri) {
            Entry::Occupied(mut e) => {
                let current = e.get().version;
                if version < 0 || version > current {
                    e.insert(DocumentEntry { rope, version });
                }
            }
            Entry::Vacant(e) => {
                e.insert(DocumentEntry { rope, version });
            }
        }
    }

    /// Remove a document and its registered URI.
    pub fn remove(&self, uri: &Url) {
        self.documents.remove(uri);
        self.registered_uris.remove(uri);
    }
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Semantic data cache: tokens, symbols, token state, and project-wide completion symbols.
#[derive(Debug)]
pub struct SymbolCache {
    pub sem_tokens: DashMap<Url, RpcSemTokensArcCell>,
    pub sem_symbols: DashMap<Url, RpcSemSymbolsArcCell>,
    pub tokens: TokensState,
    pub project_symbols: Arc<Mutex<ProjectSymbolsCache>>,
}

impl SymbolCache {
    pub fn new() -> Self {
        Self {
            sem_tokens: DashMap::new(),
            sem_symbols: DashMap::new(),
            tokens: TokensState::new(),
            project_symbols: Arc::new(Mutex::new(ProjectSymbolsCache::default())),
        }
    }
}

/// Project-level infrastructure: index worker + debounce scheduler.
#[derive(Debug)]
pub struct ProjectContext {
    pub index: IndexWorkerHandle,
    pub scheduler: ReparseScheduler,
}

impl ProjectContext {
    pub fn new(index: IndexWorkerHandle) -> Self {
        Self {
            index,
            scheduler: ReparseScheduler::new(std::time::Duration::from_millis(150)),
        }
    }
}

/// Initialization coordination: sticky flag + wakeup signal.
pub struct InitState {
    /// Sticky flag: true after project initialization completes.
    pub done: AtomicBool,
    /// Wakeup signal for tasks waiting on initialization (NOT sticky).
    pub notify: Notify,
}

impl InitState {
    pub fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    /// Mark initialization as complete and wake all waiters.
    /// Order: set flag first, then notify — so woken tasks see done == true.
    pub fn signal_done(&self) {
        self.done.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }
}

impl std::fmt::Debug for InitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitState")
            .field("done", &self.done.load(Ordering::Acquire))
            .finish()
    }
}

/// Queued diagnostics for files whose initial parse failed (e.g. mcc not yet connected).
#[derive(Debug, Default)]
pub struct DiagnosticQueue {
    /// URI → document version at open time.
    pub pending: DashMap<Url, Option<i32>>,
}

impl DiagnosticQueue {
    pub fn new() -> Self {
        Self {
            pending: DashMap::new(),
        }
    }
}

// ============================================================================
// WorkspaceState — top-level container
// ============================================================================

/// Global workspace state, composed of focused sub-structs.
pub struct WorkspaceState {
    /// Document text buffers and URI registrations.
    pub docs: DocumentStore,

    /// Semantic tokens, symbols, and project-wide completion cache.
    pub symbols: SymbolCache,

    /// Index worker + debounce scheduler.
    pub project: ProjectContext,

    /// Initialization coordination.
    pub init: InitState,

    /// Queued diagnostics awaiting retry after init.
    pub diags: DiagnosticQueue,

    /// Serializes RPC calls to mcc (single-threaded server).
    pub rpc_lock: TokioMutex<()>,
}

impl WorkspaceState {
    /// Create a state **without worker thread** (suitable for tests).
    pub fn new() -> Self {
        Self {
            docs: DocumentStore::new(),
            symbols: SymbolCache::new(),
            project: ProjectContext::new(IndexWorkerHandle::inactive()),
            init: InitState::new(),
            diags: DiagnosticQueue::new(),
            rpc_lock: TokioMutex::new(()),
        }
    }

    /// Create a state **with worker thread** (used by `Backend::new()`).
    pub fn with_worker() -> Self {
        Self {
            docs: DocumentStore::new(),
            symbols: SymbolCache::new(),
            project: ProjectContext::new(IndexWorkerHandle::spawn()),
            init: InitState::new(),
            diags: DiagnosticQueue::new(),
            rpc_lock: TokioMutex::new(()),
        }
    }

    // ── Delegated document methods (convenience, keep call sites unchanged) ──

    pub fn document_rope(&self, uri: &Url) -> Option<Rope> {
        self.docs.rope(uri)
    }

    pub fn document_version(&self, uri: &Url) -> Option<i32> {
        self.docs.version(uri)
    }

    pub fn insert_document(&self, uri: Url, rope: Rope, version: i32) {
        self.docs.insert(uri, rope, version);
    }

    pub fn remove_document(&self, uri: &Url) {
        self.docs.remove(uri);
        self.symbols.sem_tokens.remove(uri);
        self.symbols.sem_symbols.remove(uri);
        self.symbols.tokens.remove(uri);
        self.project.scheduler.remove(uri);
    }
}

// ============================================================================
// Free functions
// ============================================================================

/// After incremental text edits, shift cached lapper entry offsets so
/// F12 (goto-definition) stays accurate before the debounced reparse.
pub fn adjust_lapper_for_changes(
    state: &WorkspaceState,
    uri: &Url,
    changes: &[tower_lsp::lsp_types::TextDocumentContentChangeEvent],
    rope: &ropey::Rope,
) {
    use crate::common::position::position_to_offset;
    use crate::rpc::LapperEntry;

    let symbols_ref = match state.symbols.sem_symbols.get(uri) {
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
            None => return, // Full replace — drop all; fresh data arrives via reparse
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
