//! Use Jump — Navigate to use statement targets
//!
//! LSP entry point: `textDocument/documentLink`
//! Provides clickable links for `use` statements that navigate to the target files.

use crate::common::position::offset_to_position;
use crate::state::WorkspaceState;
use ropey::Rope;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{DocumentLink, Position, Range, Url};
use tracing::{debug, warn};

/// Resolve document links for a file.
/// Returns links for all `use` statements that point to existing files.
pub fn resolve_document_links(
    uri: &Url,
    rope: &Rope,
    state: &WorkspaceState,
) -> Option<Vec<DocumentLink>> {
    debug!("use_jump: resolving document links for {}", uri.path());

    let current_dir = uri
        .to_file_path()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))?;

    let text = rope.to_string();

    let mut links = Vec::new();

    for (line_num, line) in text.lines().enumerate() {
        let trimmed = line.trim();

        let Some(path) = strip_use_keyword(trimmed) else {
            debug!("use_jump: line {} not a use statement", line_num);
            continue;
        };

        // Parse prefix and path
        let Some((prefix, use_path)) = parse_prefix(path) else {
            debug!("use_jump: line {} has no valid prefix", line_num);
            continue;
        };

        // Only handle relative paths we can resolve
        if prefix != "./" && prefix != "../" {
            continue;
        }

        // Find target file(s)
        let candidates = resolve_use_path(&current_dir, use_path);

        let Some(target) = candidates.iter().find(|p| p.exists()) else {
            warn!("use_jump: no existing file found for line {}", line_num);
            continue;
        };

        let target_url = Url::from_file_path(target).ok()?;

        // Calculate the range of the path part (the clickable link area)
        let path_start_in_line = trimmed.find(path).unwrap_or(0);
        let path_str = format!("{}{}", prefix, use_path);
        let path_end_in_line = path_start_in_line + path_str.len();

        let range = Range::new(
            Position::new(line_num as u32, path_start_in_line as u32),
            Position::new(line_num as u32, path_end_in_line as u32),
        );

        // Find the first definition position in target file
        let (line, col) = find_first_definition(state, &target_url);

        // Tooltip shows full path
        let tooltip = format!("Jump to {}", target.to_string_lossy());

        // Build target URL with line fragment for first definition
        let final_target = Url::parse(&format!("{}#{},{}", target_url, line + 1, col + 1)).ok();

        links.push(DocumentLink {
            range,
            target: final_target,
            tooltip: Some(tooltip),
            data: None,
        });
    }

    if links.is_empty() {
        None
    } else {
        Some(links)
    }
}

/// Find the first component/interface/module definition position in target file.
/// Returns (line, column). Falls back to (0, 0) if no definition found.
fn find_first_definition(state: &WorkspaceState, target_url: &Url) -> (u32, u32) {
    let target_rope = if let Some(r) = state.document_rope(target_url) {
        r
    } else {
        // Target file not opened, try to read from disk
        let path = match target_url.to_file_path().ok() {
            Some(p) => p,
            None => return (0, 0),
        };
        let content = match std::fs::read_to_string(&path).ok() {
            Some(c) => c,
            None => return (0, 0),
        };
        return find_first_definition_in_text(&content);
    };

    let symbols_ref = match state.sem_symbols.get(target_url) {
        Some(s) => s,
        None => return (0, 0),
    };
    let symbols = match symbols_ref.lock().ok() {
        Some(s) => s,
        None => return (0, 0),
    };

    // Find the first definition (component/interface/module)
    let first_span = match symbols.local_declares.iter().min_by_key(|decl| decl.span[0]) {
        Some(s) => s,
        None => return (0, 0),
    };

    let start_offset = first_span.span[0];
    let pos = match offset_to_position(start_offset, &target_rope) {
        Some(p) => p,
        None => return (0, 0),
    };

    (pos.line, pos.character)
}

/// Find first definition position in text content (when file not in memory)
/// Returns (line, column) or (0, 0) if no definition found.
fn find_first_definition_in_text(content: &str) -> (u32, u32) {
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Look for component/interface/module definitions
        if trimmed.starts_with("component ")
            || trimmed.starts_with("interface ")
            || trimmed.starts_with("module ")
            || trimmed.starts_with("enum ")
        {
            // Find the start of the identifier
            let after_keyword = trimmed
                .split_whitespace()
                .skip(1)
                .next()
                .unwrap_or("");
            let col = line.find(after_keyword).unwrap_or(0);
            return (line_idx as u32, col as u32);
        }
    }
    (0, 0)  // Fallback to file start
}

/// Parse the use path string, returning (prefix, path_without_prefix)
fn parse_prefix(s: &str) -> Option<(&'static str, &str)> {
    if let Some(p) = s.strip_prefix("./") {
        Some(("./", p))
    } else if let Some(p) = s.strip_prefix("../") {
        Some(("../", p))
    } else {
        None
    }
}

/// Mirror mcc's parsing rules for `use <prefix><path>` to LSP side:
/// - Single-segment path `./foo` -> candidates `./foo.mc` and `./foo/foo.mc`
/// - Multi-segment path `./a/b` -> candidate `./a/b.mc`
fn resolve_use_path(base: &Path, path: &str) -> Vec<PathBuf> {
    // If path already has .mc extension, use it directly
    if path.ends_with(".mc") {
        return vec![base.join(path)];
    }

    if path.contains('/') {
        // Multi-segment: directly add .mc
        vec![base.join(format!("{path}.mc"))]
    } else {
        // Single-segment: two possibilities
        vec![
            base.join(format!("{path}.mc")),
            base.join(format!("{path}/{path}.mc")),
        ]
    }
}

/// Extract path string after `use` keyword (skip `pub` prefix and trailing comments)
fn strip_use_keyword(line: &str) -> Option<&str> {
    let after_use = line
        .strip_prefix("pub use")
        .or_else(|| line.strip_prefix("use"))?
        .trim();
    // Extract just the first word (the path), skip trailing comments
    let path = after_use.split_whitespace().next()?;
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prefixes() {
        assert_eq!(parse_prefix("./power.mc"), Some(("./", "power.mc")));
        assert_eq!(parse_prefix("../lib.mc"), Some(("../", "lib.mc")));
        assert_eq!(parse_prefix("/root.mc"), None);
        assert_eq!(parse_prefix("$sys.mc"), None);
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
    fn find_first_definition_in_text_finds_component() {
        let content = r#"# Comment
pub use ./other.mc

component AMP {}
"#;
        let (line, col) = find_first_definition_in_text(content);
        assert_eq!(line, 3); // 0-indexed, line 4
    }

    #[test]
    fn find_first_definition_in_text_no_cmie_returns_zero() {
        let content = r#"# Comment only
pub use ./other.mc
"#;
        let (line, col) = find_first_definition_in_text(content);
        assert_eq!(line, 0);
        assert_eq!(col, 0);
    }
}
