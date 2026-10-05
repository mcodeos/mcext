//! Common utilities
//!
//! - [`usechk`]: shared use-path parsing + pre-validation that target files exist
//!   before calling mcc RPCs (prevents SIGSEGV from null deref on missing paths).

pub mod usechk;

pub use usechk::{
    check_use_targets, parse_use_prefix, resolve_use_path, resolve_use_target, strip_use_keyword,
    UseCheckResult,
};

/// Decoded filesystem path for a document URI, for everything handed to mcc.
///
/// `Url::path()` keeps the percent-encoding (`libs/x@0.4/` arrives as
/// `libs/x%400.4/`), and mcc stats the string verbatim — the encoded form
/// matches no file, so add_file/sem/diagnostics silently fail with
/// "file not found in workspace" for any path containing `@`, spaces or
/// non-ASCII. Always route mcc-facing paths through this.
pub fn uri_fs_path(uri: &tower_lsp::lsp_types::Url) -> String {
    uri.to_file_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| uri.path().to_string())
}
