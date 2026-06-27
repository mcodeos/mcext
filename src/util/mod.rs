//! Common utilities
//!
//! - [`use_check`]: pre-validate that `use` target files exist before calling `mcc::mcc_add`,
//!   prevents mcc C library from null deref triggering SIGSEGV for non-existent paths.

pub mod use_check;

pub use use_check::{check_use_targets, UseCheckResult};
