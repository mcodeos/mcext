//! Common utilities
//!
//! - [`usechk`]: pre-validate that `use` target files exist before calling `mcc::mcc_add`,
//!   prevents mcc C library from null deref triggering SIGSEGV for non-existent paths.

pub mod usechk;

pub use usechk::{check_use_targets, UseCheckResult};
