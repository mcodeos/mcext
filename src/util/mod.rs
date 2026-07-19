//! Common utilities
//!
//! - [`usechk`]: shared use-path parsing + pre-validation that target files exist
//!   before calling mcc RPCs (prevents SIGSEGV from null deref on missing paths).

pub mod usechk;

pub use usechk::{
    check_use_targets, parse_use_prefix, resolve_use_path, resolve_use_target, strip_use_keyword,
    UseCheckResult,
};
