//! Use Jump — Navigate to use statement targets
//!
//! LSP entry point: `textDocument/documentLink`
//! Provides clickable links for `use` statements that navigate to the target files.

use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{DocumentLink, Position, Range, Url};
use tracing::{debug, info, warn};

/// Resolve document links for a file.
/// Returns links for all `use` statements that point to existing files.
pub fn resolve_document_links(uri: &Url, text: &str) -> Option<Vec<DocumentLink>> {
    info!("use_jump: resolving document links for {}", uri.path());

    let current_dir = uri
        .to_file_path()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))?;

    info!("use_jump: current_dir = {:?}", current_dir);

    let mut links = Vec::new();

    for (line_num, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        info!("use_jump: line {}: {:?}", line_num, trimmed);

        let Some(path) = strip_use_keyword(trimmed) else {
            debug!("use_jump: line {} not a use statement", line_num);
            continue;
        };

        info!("use_jump: after strip_use_keyword: path = {:?}", path);

        // Parse prefix and path
        let Some((prefix, use_path)) = parse_prefix(path) else {
            debug!("use_jump: line {} has no valid prefix", line_num);
            continue;
        };

        info!("use_jump: prefix = {:?}, use_path = {:?}", prefix, use_path);

        // Only handle relative paths we can resolve
        if prefix != "./" && prefix != "../" {
            debug!(
                "use_jump: line {} has unsupported prefix: {}",
                line_num, prefix
            );
            continue;
        }

        // Find target file(s)
        let candidates = resolve_use_path(&current_dir, use_path);
        info!("use_jump: candidates = {:?}", candidates);

        let Some(target) = candidates.iter().find(|p| p.exists()) else {
            warn!("use_jump: no existing file found for line {}", line_num);
            continue;
        };

        info!("use_jump: found target = {:?}", target);

        // Calculate the range of the path part (after the prefix)
        // e.g., for "use ./power.mc", range is from "./power.mc" part
        let path_start_in_line = trimmed.find(path).unwrap_or(0);
        let path_str = format!("{}{}", prefix, use_path);
        let path_end_in_line = path_start_in_line + path_str.len();

        info!(
            "use_jump: range: start={}, end={}, path_str={}",
            path_start_in_line, path_end_in_line, path_str
        );

        let range = Range::new(
            Position::new(line_num as u32, path_start_in_line as u32),
            Position::new(line_num as u32, path_end_in_line as u32),
        );

        let target_url = Url::from_file_path(target).ok()?;
        info!("use_jump: target_url = {:?}", target_url);

        links.push(DocumentLink {
            range,
            target: Some(target_url),
            data: None,
            tooltip: Some(format!("Jump to {}", path_str)),
        });
    }

    info!("use_jump: total links = {}", links.len());

    if links.is_empty() {
        None
    } else {
        Some(links)
    }
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
}
