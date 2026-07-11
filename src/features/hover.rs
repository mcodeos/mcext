//! Hover — Hover information for use statements
//!
//! LSP entry point: `textDocument/hover`
//! Shows all component/interface/module definitions when hovering over a use statement.

use crate::state::WorkspaceState;
use tower_lsp::lsp_types::{Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, Url};

/// Maximum number of definitions to display
const MAX_DEFINITIONS: usize = 7;

/// Resolve hover information for a position.
pub fn resolve(
    state: &WorkspaceState,
    params: &HoverParams,
) -> Option<Hover> {
    let uri = &params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;

    let rope = state.document_rope(uri)?;
    let offset = crate::common::position::position_to_offset(position, &rope)?;

    // Get current line
    let line_idx = rope.try_byte_to_line(offset).ok()?;
    let line_text = rope.get_line(line_idx)?.to_string();
    let trimmed = line_text.trim();

    // Check if this is a use statement
    let Some(path) = strip_use_keyword(trimmed) else {
        return None;
    };

    // Parse prefix and path
    let Some((prefix, use_path)) = parse_prefix(path) else {
        return None;
    };

    // Only handle relative paths
    if prefix != "./" && prefix != "../" {
        return None;
    }

    // Get current file directory
    let current_file = uri.to_file_path().ok()?;
    let current_dir = current_file.parent()?;

    // Resolve target path
    let candidates = resolve_use_path(&current_dir, use_path);
    let target = candidates.iter().find(|p| p.exists())?;
    let target_url = Url::from_file_path(target).ok()?;

    // Find all definitions in target file
    let definitions = find_all_definitions(state, &target_url);
    let path_str = target.to_string_lossy();

    // Build hover content - use Markdown code block for correct formatting
    let content = if definitions.is_empty() {
        HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```\n{}\n```", path_str),
        })
    } else {
        let defs: Vec<String> = definitions
            .iter()
            .take(MAX_DEFINITIONS)
            .map(|s| {
                s.replace('\r', "")
                    .replace('\n', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|s| !s.is_empty())
            .collect();

        let mut value = path_str.to_string();
        value.push('\n');
        for def in &defs {
            value.push('\n');
            value.push_str(def);
        }
        if definitions.len() > MAX_DEFINITIONS {
            value.push_str("\n...");
        }

        HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```\n{}\n```", value),
        })
    };

    Some(Hover {
        contents: content,
        range: None,
    })
}

/// Find all component/interface/module definitions in target file.
fn find_all_definitions(state: &WorkspaceState, target_url: &Url) -> Vec<String> {
    // Try to get rope from state or disk
    let content = if let Some(r) = state.document_rope(target_url) {
        r.to_string()
    } else {
        let path = match target_url.to_file_path().ok() {
            Some(p) => p,
            None => return Vec::new(),
        };
        match std::fs::read_to_string(&path).ok() {
            Some(c) => c,
            None => return Vec::new(),
        }
    };

    extract_all_signatures_from_text(&content)
}

/// Extract all definition signatures from text content, handling multi-line definitions
fn extract_all_signatures_from_text(content: &str) -> Vec<String> {
    let mut defs = Vec::new();
    let mut in_def = false;
    let mut current_def = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("component ")
            || trimmed.starts_with("interface ")
            || trimmed.starts_with("module ")
        {
            // Start of a new definition
            if in_def {
                push_def(&mut defs, &current_def);
            }
            current_def = trimmed.to_string();
            in_def = true;

            // If this line already has `{`, the definition is complete on one line
            if trimmed.contains('{') {
                push_def(&mut defs, &current_def);
                current_def.clear();
                in_def = false;
            }
        } else if in_def {
            // Continuation of a multi-line definition
            current_def.push(' ');
            current_def.push_str(trimmed);

            // Stop when we hit `{` or `)`, whichever comes first
            if trimmed.contains('{') || trimmed.ends_with(')') || trimmed.ends_with(")\n")
            {
                push_def(&mut defs, &current_def);
                current_def.clear();
                in_def = false;
            }
        }
    }

    // Don't forget any remaining
    if in_def && !current_def.is_empty() {
        push_def(&mut defs, &current_def);
    }

    defs
}

fn push_def(defs: &mut Vec<String>, raw: &str) {
    let collapsed = raw
        .replace('\r', "")
        .replace('{', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !collapsed.is_empty() {
        defs.push(collapsed);
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

/// Resolve use path to candidate file paths
fn resolve_use_path(base: &std::path::Path, path: &str) -> Vec<std::path::PathBuf> {
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

/// Extract path string after `use` keyword
fn strip_use_keyword(line: &str) -> Option<&str> {
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
