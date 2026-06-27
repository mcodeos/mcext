//! Project-level symbol index
//!
//! Cross-file jump / reference lookup requires global view:
//! - Current open document's `McSemSymbols` only contains this file's info
//! - Cross-file jump target URI + span comes from `global_table` (shared by all files)
//!
//! This module maintains a **ProjectIndex**: caches metadata of all registered files'
//! `(uri, span, name)`, for `features::goto_definition` /
//! `features::references` to query when current file misses.
//!
//! ## Data flow
//!
//! ```text
//! did_open / did_change / workspace/didChangeWatchedFiles
//!     │
//! ▼
//! IndexCommand → tokio task
//!     │
//! ▼
//! Analyzer (sync mcc_* in spawn_blocking)
//!     │
//! ▼
//! ProjectIndex snapshot → broadcast to subscribers via watch channel
//! ```

pub mod snapshot;
pub mod worker;

pub use snapshot::{IndexEntry, IndexKind, ProjectIndex};
pub use worker::{IndexCommand, IndexWorkerHandle};
