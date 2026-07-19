//! Hover — Show symbol information on hover
//!
//! LSP entry point: `textDocument/hover`
//!
//! Two modes:
//!   (1) Use-statement hover — shows component/interface/module/enum definitions
//!       in the target file, using the project index.
//!   (2) Symbol hover — shows definition info for any symbol tracked by
//!       the semantic lapper (class, port, label, enum value, func, etc.).
//!
//! Shares data sources with gotodef: sem-symbols lapper + project index.

use crate::index::snapshot::{IndexEntry, IndexKind};
use crate::state::WorkspaceState;
use crate::util::usechk::{parse_use_prefix, resolve_use_target, strip_use_keyword};
use ropey::Rope;
use tower_lsp::lsp_types::{Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, Url};

/// Maximum number of definition entries to display in a hover tooltip.
const MAX_ENTRIES: usize = 8;

// ============================================================================
// Public entry point
// ============================================================================

/// Resolve hover information for a position.
pub fn resolve(state: &WorkspaceState, params: &HoverParams) -> Option<Hover> {
    let uri = &params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;
    let rope = state.document_rope(uri)?;
    let offset = crate::common::position::position_to_offset(position, &rope)?;

    // ── (1) Use-statement hover ──
    if let Some(hover) = resolve_use_hover(&rope, offset, uri, state) {
        return Some(hover);
    }

    // ── (2) Symbol hover ──
    if let Some(hover) = resolve_symbol_hover(state, uri, &rope, offset) {
        return Some(hover);
    }

    None
}

// ============================================================================
// (1) Use-statement hover
// ============================================================================

/// Build hover for `use ./path` statements — list all public definitions
/// in the target file using the project index snapshot.
fn resolve_use_hover(
    rope: &Rope,
    offset: usize,
    uri: &Url,
    state: &WorkspaceState,
) -> Option<Hover> {
    let line_idx = rope.try_byte_to_line(offset).ok()?;
    let line_text = rope.get_line(line_idx)?.to_string();
    let trimmed = line_text.trim();

    // Only trigger on use / pub use lines
    let path_str = strip_use_keyword(trimmed)?;
    let (_prefix, use_path) = parse_use_prefix(path_str)?;

    // Only handle relative paths for now
    if !path_str.starts_with("./") && !path_str.starts_with("../") {
        return None;
    }

    // Resolve target URL
    let target_url = resolve_use_target(uri, use_path)?;

    // Query index for all definitions in the target file
    let entries = lookup_file_entries(state, &target_url);

    // Build hover content
    let file_label = target_url
        .to_file_path()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| target_url.to_string());

    let content = if entries.is_empty() {
        format_markdown(&format!("📁 `{}`", file_label), &[])
    } else {
        let def_lines: Vec<String> = entries
            .iter()
            .take(MAX_ENTRIES)
            .map(|e| format_entry_line(e))
            .collect();

        let header = format!("📁 `{}`  — {} definition(s)", file_label, entries.len());
        format_markdown(&header, &def_lines)
    };

    Some(Hover {
        contents: HoverContents::Markup(content),
        range: None,
    })
}

/// Collect all index entries whose URI matches the target file.
fn lookup_file_entries(state: &WorkspaceState, target_url: &Url) -> Vec<IndexEntry> {
    let snap = state.index.snapshot();
    snap.lookup_file(target_url)
        .into_iter()
        .map(|(_kind, entry)| entry.clone())
        .collect()
}

// ============================================================================
// (2) Symbol hover
// ============================================================================

/// Build hover for a semantic symbol at the cursor position.
///
/// Uses the shared `find_symbol_at_offset` (same data source as gotodef) to
/// find which symbol is under the cursor, then looks up its definition in the
/// project index or cross-file-targets table.
fn resolve_symbol_hover(
    state: &WorkspaceState,
    uri: &Url,
    _rope: &Rope,
    offset: usize,
) -> Option<Hover> {
    let (info, name) = crate::features::symbols::find_symbol_at_offset(state, uri, offset)?;

    match info.kind.as_str() {
        // Self-defining symbols — show their type
        "class_def" | "class_definition" | "function_def" | "port_def" | "define_def"
        | "role_def" | "enum_class_def" | "enum_value_def" | "pin_name_def" | "label_def" => {
            format_symbol_hover(&name, &info.kind_label, &info.scope)
        }

        // Reference symbols — try to resolve to definition
        "class_ref" | "declare_class" | "instance_ref" | "interface_ref" | "enum_class_ref"
        | "enum_value_ref" | "function_ref" | "pin_name_ref" | "label_ref" => {
            resolve_reference_hover(state, &name, info.kind.as_str(), &info.scope)
        }

        // Instance definitions / declarations
        "instance_def" | "declare_instance" => format_symbol_hover(&name, "instance", &info.scope),

        _ => None,
    }
}

/// Resolve a reference (ref kind) to its definition for hover display.
fn resolve_reference_hover(
    state: &WorkspaceState,
    name: &str,
    kind: &str,
    scope: &str,
) -> Option<Hover> {
    let snap = state.index.snapshot();

    // Determine the index kind to search
    let index_kind = match kind {
        "class_ref" | "declare_class" => Some(IndexKind::Component),
        "interface_ref" => Some(IndexKind::Interface),
        "enum_class_ref" => Some(IndexKind::Enum),
        _ => None,
    };

    // Try index lookup first
    if let Some(ik) = index_kind {
        let entries = snap.lookup(ik, name);
        if let Some(entry) = entries.first() {
            let source = entry
                .uri
                .to_file_path()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_default();
            let ref_kind = kind_label(kind);
            let lines = vec![
                format!("→ `{}` ({})", name, ref_kind),
                format!("📄 {}", source),
            ];
            return format_markdown_hover(&lines);
        }
    }

    // Fallback: show scope + name
    if !scope.is_empty() {
        let lines = vec![
            format!("`{}` — reference", name),
            format!("scope: `{}`", scope),
        ];
        format_markdown_hover(&lines)
    } else {
        format_symbol_hover(name, kind_label(kind), "")
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Human-readable label for a lapper kind string.
fn kind_label<'a>(kind: &'a str) -> &'a str {
    match kind {
        "class_def" | "class_definition" => "component/module",
        "class_ref" | "declare_class" => "→ class",
        "port_def" => "port",
        "label_def" => "label",
        "label_ref" => "→ label",
        "function_def" => "function",
        "function_ref" => "→ function",
        "pin_name_def" => "pin",
        "pin_name_ref" => "→ pin",
        "enum_class_def" | "enum_value_def" => "enum",
        "enum_class_ref" | "enum_value_ref" => "→ enum",
        "instance_def" | "declare_instance" => "instance",
        "instance_ref" => "→ instance",
        "define_def" => "define",
        "role_def" => "role",
        "interface_ref" => "→ interface",
        _ => kind,
    }
}

/// Build a hover for a self-defining symbol.
fn format_symbol_hover(name: &str, kind: &str, scope: &str) -> Option<Hover> {
    let mut lines = vec![format!("`{}` — {}", name, kind)];
    if !scope.is_empty() {
        lines.push(format!("scope: `{}`", scope));
    }
    format_markdown_hover(&lines)
}

/// Format a single index entry as a human-readable line.
fn format_entry_line(entry: &IndexEntry) -> String {
    let span_info = format!("[{}:{}]", entry.span.0, entry.span.1);
    format!("{}  {}", entry.name, span_info)
}

// ── Markdown formatting ──

fn format_markdown(header: &str, lines: &[String]) -> MarkupContent {
    let mut value = header.to_string();
    for line in lines {
        value.push('\n');
        value.push_str(line);
    }
    MarkupContent {
        kind: MarkupKind::Markdown,
        value,
    }
}

fn format_markdown_hover(lines: &[String]) -> Option<Hover> {
    if lines.is_empty() {
        return None;
    }
    let value = lines.join("\n");
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    })
}

// Use-statement path helpers are in crate::util::usechk.
