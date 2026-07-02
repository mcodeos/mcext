//! Project index worker
//!
//! Maintains `ProjectIndex` in a background thread:
//! - At startup, traverses project root and calls RPC to load all .mc at once
//! - Afterwards responds to `AddFile` / `RemoveFile` for single file add/remove
//!
//! Broadcasts new snapshots to LSP handlers via `watch::channel`.

use super::snapshot::{build_from_mcb_iter, ProjectIndex};
use crate::rpc::SymbolEntry;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tracing::debug;

/// Index worker commands
#[derive(Debug, Clone)]
pub enum IndexCommand {
    /// Traverse entire project root at startup
    ParseAll(PathBuf),
    /// Add single file (after did_open)
    AddFile(String),
    /// Remove single file (did_close or file deleted)
    RemoveFile(String),
    /// Update project symbols (from RPC)
    UpdateProjectSymbols {
        components: Vec<SymbolEntry>,
        interfaces: Vec<SymbolEntry>,
        enums: Vec<SymbolEntry>,
        modules: Vec<SymbolEntry>,
    },
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

    /// Inactive handle (for tests)
    pub fn inactive() -> Self {
        Self { inner: None }
    }

    pub fn send(&self, cmd: IndexCommand) -> Result<(), mpsc::error::SendError<IndexCommand>> {
        match &self.inner {
            Some(inner) => inner.tx.send(cmd),
            None => Ok(()),
        }
    }

    pub fn snapshot(&self) -> ProjectIndex {
        match &self.inner {
            Some(inner) => inner.snapshot_rx.borrow().clone(),
            None => ProjectIndex::new(),
        }
    }
}

/// Worker main loop (in separate thread)
fn worker_loop(
    mut rx: mpsc::UnboundedReceiver<IndexCommand>,
    snap_tx: watch::Sender<ProjectIndex>,
) {
    // ProjectIndex is updated via UpdateProjectSymbols command
    #[allow(unused_assignments)]
    let mut current: ProjectIndex;

    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            IndexCommand::ParseAll(_root) => {
                // Project symbols will be updated via UpdateProjectSymbols command
                debug!("worker: ParseAll received, waiting for project symbols");
            }
            IndexCommand::AddFile(_uri) => {
                debug!("worker: AddFile received");
                // Project symbols will be updated via UpdateProjectSymbols command
            }
            IndexCommand::RemoveFile(_uri) => {
                debug!("worker: RemoveFile received");
                // Project symbols will be updated via UpdateProjectSymbols command
            }
            IndexCommand::UpdateProjectSymbols {
                components,
                interfaces,
                enums,
                modules,
            } => {
                debug!(
                    "worker: UpdateProjectSymbols components={} interfaces={} enums={} modules={}",
                    components.len(),
                    interfaces.len(),
                    enums.len(),
                    modules.len()
                );
                let components_tuples: Vec<_> =
                    components.into_iter().map(|c| (c.name, c.uri)).collect();
                let interfaces_tuples: Vec<_> =
                    interfaces.into_iter().map(|i| (i.name, i.uri)).collect();
                let enums_tuples: Vec<_> = enums.into_iter().map(|e| (e.name, e.uri)).collect();
                let modules_tuples: Vec<_> = modules.into_iter().map(|m| (m.name, m.uri)).collect();

                current = build_from_mcb_iter(
                    None,
                    components_tuples,
                    interfaces_tuples,
                    enums_tuples,
                    modules_tuples,
                );
                let _ = snap_tx.send(current.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_command() {
        let handle = IndexWorkerHandle::inactive();
        let cmd = IndexCommand::ParseAll(PathBuf::from("/test"));
        // Should not panic
        let _ = handle.send(cmd);
    }

    #[test]
    fn inactive_handle_snapshot() {
        let handle = IndexWorkerHandle::inactive();
        let idx = handle.snapshot();
        assert!(idx.is_empty());
    }
}
