//! Project index worker
//!
//! Maintains `ProjectIndex` in a background thread:
//! - At startup, traverses project root and calls `mcc::mcc_load_project` to load all .mc at once
//! - Afterwards responds to `AddFile` / `RemoveFile` for single file add/remove
//!
//! Broadcasts new snapshots to LSP handlers via `watch::channel`.
//!
//! **Note**: Phase 1 primarily uses it to establish cross-file jump data foundation. Specifically,
//! `(uri, span)` → `Range` conversion in `features::goto_definition` reads target files from disk
//! on demand (to avoid caching all file contents in memory).

use super::snapshot::{build_from_mcb_iter, ProjectIndex};
use mcc::McURI;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tracing::{debug, error, warn};

/// Index worker commands
#[derive(Debug, Clone)]
pub enum IndexCommand {
    /// Traverse entire project root at startup
    ParseAll(PathBuf),
    /// Add single file (after did_open)
    AddFile(McURI),
    /// Remove single file (did_close or file deleted)
    RemoveFile(McURI),
}

/// Worker external handle
#[derive(Clone)]
pub struct IndexWorkerHandle {
    inner: Option<InnerHandle>,
}

#[derive(Clone)]
struct InnerHandle {
    tx: mpsc::UnboundedSender<IndexCommand>,
    snapshot_rx: watch::Receiver<ProjectIndex>,
}

impl IndexWorkerHandle {
    /// Start a new worker thread
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (snap_tx, snap_rx) = watch::channel(ProjectIndex::new());

        std::thread::spawn(move || {
            worker_loop(rx, snap_tx);
        });

        Self {
            inner: Some(InnerHandle {
                tx,
                snapshot_rx: snap_rx,
            }),
        }
    }

    /// Create a handle that **does not spawn a thread** (for testing / single-threaded scenarios)
    ///
    /// All send / snapshot calls degenerate to no-op.
    pub fn inactive() -> Self {
        Self { inner: None }
    }

    /// Send command (non-blocking)
    pub fn send(&self, cmd: IndexCommand) -> Result<(), mpsc::error::SendError<IndexCommand>> {
        match &self.inner {
            Some(i) => i.tx.send(cmd),
            None => Ok(()),
        }
    }

    /// Get current snapshot
    pub fn snapshot(&self) -> ProjectIndex {
        match &self.inner {
            Some(i) => i.snapshot_rx.borrow().clone(),
            None => ProjectIndex::new(),
        }
    }

    /// Wait for next snapshot update
    pub async fn wait_next(&mut self) -> Option<ProjectIndex> {
        match &mut self.inner {
            Some(i) => {
                i.snapshot_rx.changed().await.ok()?;
                Some(i.snapshot_rx.borrow().clone())
            }
            None => None,
        }
    }
}

impl Default for IndexWorkerHandle {
    fn default() -> Self {
        Self::inactive()
    }
}

/// Worker main loop (in separate thread, avoid blocking tokio)
fn worker_loop(
    mut rx: mpsc::UnboundedReceiver<IndexCommand>,
    snap_tx: watch::Sender<ProjectIndex>,
) {
    let mut current = ProjectIndex::new();

    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            IndexCommand::ParseAll(root) => {
                current = rebuild_all(&root);
                let _ = snap_tx.send(current.clone());
            }
            IndexCommand::AddFile(uri) => {
                // Single file add: directly use mcc::mcc_add, then do a lightweight incremental update
                mcc::mcc_add(&uri);
                if let Some(root) = current.project_root.clone() {
                    // Simple strategy: rebuild entire project (mcc itself is already ~O(n))
                    current = rebuild_all(&root);
                    let _ = snap_tx.send(current.clone());
                }
            }
            IndexCommand::RemoveFile(uri) => {
                mcc::mcc_remove(&uri);
                if let Ok(url) = url_from_uri(&uri) {
                    current.remove_file(&url);
                    let _ = snap_tx.send(current.clone());
                }
            }
        }
    }
}

/// Rebuild project-level index
///
/// **Thread-safe**: all mcc_* calls are serialized under `crate::mcc_lock::MCC_LOCK` protection,
/// wrapped with `catch_unwind` to prevent worker thread death from mcc panic.
fn rebuild_all(root: &std::path::Path) -> ProjectIndex {
    let root = root.to_path_buf();

    let mcc_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = crate::mcc_lock::MCC_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        mcc::mcc_set_project_root(&root);

        // Find entry point: main.mc in root, or first .mc file
        // mcc_load_project should be called ONCE with entry point, not for each file
        let mc_files = walk_mc_files(&root);
        if mc_files.is_empty() {
            return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        }

        // Check use targets for each file (warning only)
        for mc_path in &mc_files {
            let text = std::fs::read_to_string(mc_path).unwrap_or_default();
            let url = tower_lsp::lsp_types::Url::from_file_path(mc_path)
                .unwrap_or_else(|_| tower_lsp::lsp_types::Url::parse("file:///").unwrap());
            if let crate::util::UseCheckResult::Missing { use_line, .. } =
                crate::util::check_use_targets(&url, &text)
            {
                let uri = McURI::from(mc_path.to_string_lossy().to_string());
                warn!("worker: use target missing for {uri}: {use_line}");
            }
        }

        // Find entry point: prefer main.mc, fall back to first file
        let entry_path = mc_files
            .iter()
            .find(|p| p.file_name().map_or(false, |n| n == "main.mc"))
            .or_else(|| mc_files.first())
            .cloned();

        if let Some(entry_path) = entry_path {
            let entry_uri = McURI::from(entry_path.to_string_lossy().to_string());
            debug!("worker: loading project with entry: {}", entry_uri);
            mcc::mcc_load_project(&entry_uri);
        }

        let components = mcc::mcb_iter_components();
        let interfaces = mcc::mcb_iter_interfaces();
        let enums = mcc::mcb_iter_enums();
        let modules = mcc::mcb_iter_modules();
        (components, interfaces, enums, modules)
    }));

    match mcc_result {
        Ok((components, interfaces, enums, modules)) => {
            build_from_mcb_iter(Some(root), components, interfaces, enums, modules)
        }
        Err(panic_payload) => {
            let msg = if let Some(s) = panic_payload.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else {
                "unknown panic payload".into()
            };
            error!("mcc panic during rebuild_all: {}", msg);
            ProjectIndex::new()
        }
    }
}

/// Recursively scan all `.mc` files in a directory
///
/// Skips `.git` / `target` / `node_modules` / `.vscode` to avoid pollution.
/// Limits to 8 levels of depth to prevent abnormal recursion (e.g., symbolic links).
fn walk_mc_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    walk_recursive(root, &mut out, 8);
    out
}

fn walk_recursive(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth == 0 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | "target" | "node_modules" | ".vscode") {
                    continue;
                }
            }
            walk_recursive(&path, out, depth - 1);
        } else if path.extension().is_some_and(|e| e == "mc") {
            out.push(path);
        }
    }
}

/// mcc::McURI = String → LSP URL
fn url_from_uri(uri: &McURI) -> Result<tower_lsp::lsp_types::Url, ()> {
    tower_lsp::lsp_types::Url::from_file_path(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_from_uri_works() {
        let uri = McURI::from("/tmp/test.mc".to_string());
        let url = url_from_uri(&uri).unwrap();
        assert!(url.as_str().contains("test.mc"));
    }
}
