//! `use` path utilities — parsing, resolution, and pre-validation
//!
//! Provides shared `use`-directive helpers used across features (gotodef, hover, completion)
//! and the pre-validation step that guards against mcc SIGSEGV on missing target files.
//!
//! ## Why pre-validation is needed
//!
//! mcc's `use` parser auto-completes single-segment paths (`./helper` → `./helper/helper.mc`).
//! When the target file doesn't exist, mcc does a null deref → SIGSEGV.
//! `std::panic::catch_unwind` can't catch OS signals, so the entire LSP process would be killed.
//!
//! ## Solution
//!
//! Before calling mcc RPCs, scan the document for `use` directives and verify target files exist.
//! No match → skip the call and report diagnostics to the user.
//!
//! ## Long-term solution
//!
//! mcc already runs as an isolated subprocess via RPC (see `mccsrv.rs`).
//! SIGSEGV only kills the child process, not LSP.

use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::Url;

/// Pre-validation result
#[derive(Debug, Clone)]
pub enum UseCheckResult {
    /// All use targets exist, safe to call mcc RPC
    Ok,
    /// Some use targets missing, skip mcc call and notify user
    Missing {
        use_line: String,
        candidates: Vec<PathBuf>,
    },
}

// ============================================================================
// Shared use-path helpers (used by gotodef, hover, etc.)
// ============================================================================

/// Strip `use` / `pub use` prefix from a line and return the path portion.
///
/// Returns `None` if the line does not start with `use` or `pub use`.
///
/// # Examples
///
/// ```
/// assert_eq!(strip_use_keyword("use ./helper"), Some("./helper"));
/// assert_eq!(strip_use_keyword("pub use ./helper as h"), Some("./helper"));
/// assert_eq!(strip_use_keyword("not a use"), None);
/// ```
pub fn strip_use_keyword(line: &str) -> Option<&str> {
    let after_use = line
        .strip_prefix("pub use")
        .or_else(|| line.strip_prefix("use"))?
        .trim();
    let path = after_use.split_whitespace().next()?;
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Split a prefix (`./` or `../`) from the path string.
///
/// Returns `(prefix, rest)` or `None` for non-relative paths.
pub fn parse_use_prefix(s: &str) -> Option<(&'static str, &str)> {
    if let Some(p) = s.strip_prefix("./") {
        Some(("./", p))
    } else if let Some(p) = s.strip_prefix("../") {
        Some(("../", p))
    } else {
        None
    }
}

/// Mirror mcc's parsing rules for `use <prefix><path>` to LSP side.
///
/// - Single-segment path `./foo` → candidates `./foo.mc` and `./foo/foo.mc`
/// - Multi-segment path `./a/b` → candidate `./a/b.mc`
/// - Path already ending in `.mc` → returned as-is
pub fn resolve_use_path(base: &Path, path: &str) -> Vec<PathBuf> {
    if path.ends_with(".mc") {
        return vec![base.join(path)];
    }
    if path.contains('/') {
        vec![base.join(format!("{path}.mc"))]
    } else {
        vec![
            base.join(format!("{path}.mc")),
            base.join(format!("{path}/{path}.mc")),
        ]
    }
}

/// Resolve a use-path string against a document URI, returning the
/// first existing target as a `file://` URL.
///
/// Convenience wrapper that combines directory extraction, candidate generation,
/// and disk existence check.
pub fn resolve_use_target(base_url: &Url, use_path: &str) -> Option<Url> {
    let current_file = base_url.to_file_path().ok()?;
    let current_dir = current_file.parent()?;
    let target = resolve_use_path(current_dir, use_path)
        .iter()
        .find(|p| p.exists())?
        .clone();
    Url::from_file_path(target).ok()
}

// ============================================================================
// Pre-validation
// ============================================================================

/// Scan all `use` directives in document, check if their target files exist on disk.
///
/// Only checks relative paths (`./xxx`, `../xxx`). System libraries (`$xxx`) and
/// project-root paths (`/xxx`) are skipped (we can't predict their locations here).
pub fn check_use_targets(uri: &Url, text: &str) -> UseCheckResult {
    let Some(current_dir) = uri
        .to_file_path()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    else {
        return UseCheckResult::Ok; // Can't resolve path, give up checking
    };

    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = strip_use_keyword(trimmed) else {
            continue;
        };

        // Parse prefix — inline version handling all 4 cases for validation
        let (prefix, path) = if let Some(p) = rest.strip_prefix("./") {
            ("./", p)
        } else if let Some(p) = rest.strip_prefix("../") {
            ("../", p)
        } else if let Some(p) = rest.strip_prefix("/") {
            ("/", p) // Project root, needs project_root config; can't check here
        } else if let Some(p) = rest.strip_prefix("$") {
            ("$", p) // System library, can't check
        } else {
            continue;
        };

        if prefix == "/" || prefix == "$" {
            continue; // Skip paths we can't validate
        }

        // Extract path part (remove suffixes like `as xxx` or `import(...)`)
        let path = path
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(|c: char| [',', ';'].contains(&c));

        // Remove trailing `.mc` — resolve_use_path will add it again
        let path = path.trim_end_matches(".mc");

        let candidates = resolve_use_path(&current_dir, path);

        if !candidates.iter().any(|p| p.exists()) {
            return UseCheckResult::Missing {
                use_line: format!("use {prefix}{path}"),
                candidates,
            };
        }
    }

    UseCheckResult::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(p: &str) -> Url {
        Url::parse(p).unwrap()
    }

    #[test]
    fn missing_file_returns_missing() {
        let uri = url("file:///tmp/nonexistent_dir/main.mc");
        let text = "use ./helper\n";
        let result = check_use_targets(&uri, text);
        assert!(matches!(result, UseCheckResult::Missing { .. }));
    }

    #[test]
    fn system_lib_skipped() {
        let uri = url("file:///tmp/main.mc");
        let result = check_use_targets(&uri, "use $conn\n");
        assert!(matches!(result, UseCheckResult::Ok));
    }

    #[test]
    fn project_root_skipped() {
        let uri = url("file:///tmp/main.mc");
        let result = check_use_targets(&uri, "use /helper\n");
        assert!(matches!(result, UseCheckResult::Ok));
    }

    #[test]
    fn pub_use_works() {
        let uri = url("file:///tmp/nonexistent_dir/main.mc");
        let text = "pub use ./helper\n";
        let result = check_use_targets(&uri, text);
        assert!(matches!(result, UseCheckResult::Missing { .. }));
    }

    #[test]
    fn resolve_use_path_single_segment() {
        let base = Path::new("/tmp/test");
        let candidates = resolve_use_path(base, "helper");
        assert_eq!(candidates.len(), 2);
        assert!(candidates[0].ends_with("helper.mc"));
        assert!(candidates[1].ends_with("helper/helper.mc"));
    }

    #[test]
    fn resolve_use_path_multi_segment() {
        let base = Path::new("/tmp/test");
        let candidates = resolve_use_path(base, "a/b");
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].ends_with("a/b.mc"));
    }

    #[test]
    fn strip_use_keyword_handles_pub() {
        assert_eq!(strip_use_keyword("use ./helper"), Some("./helper"));
        assert_eq!(strip_use_keyword("pub use ./helper"), Some("./helper"));
        assert_eq!(strip_use_keyword("use ./helper as h"), Some("./helper"));
        assert_eq!(strip_use_keyword("not use"), None);
    }

    #[test]
    fn real_file_check_passes() {
        // 用当前源文件做测试
        let uri = url("file:///");
        let text = std::fs::read_to_string(file!()).unwrap();
        // 我们这个文件没有 use 指令 → Ok
        let result = check_use_targets(&uri, &text);
        assert!(matches!(result, UseCheckResult::Ok));
    }
}
