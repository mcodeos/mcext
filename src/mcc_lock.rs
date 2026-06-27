//! Global mcc mutex
//!
//! mcc crate internally uses global state, **not thread-safe**. All `mcc::*` calls must first acquire
//! this lock, preventing SIGABRT from concurrent access between LSP handler (tokio task) and
//! index worker (std::thread).
//!
//! ## Usage
//!
//! ```ignore
//! use crate::mcc_lock::MCC_LOCK;
//!
//! let _guard = MCC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
//! mcc::mcc_add(&uri);  // <- now safe
//! ```

use std::sync::Mutex;

/// Global mcc mutex. All mcc_* calls must first acquire.
///
/// Uses `std::sync::Mutex` (sync) instead of `tokio::sync::Mutex`:
/// - worker is std::thread, can't use tokio::sync
/// - server's async handler acquires and immediately releases (doesn't cross await), blocking time < 1ms
pub static MCC_LOCK: Mutex<()> = Mutex::new(());
