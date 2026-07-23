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
    let snap = state.project.index.snapshot();
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

    match info.kind {
        // Self-defining symbols — show their type
        0 | 8 | 4 | 22 | 20 | 16 | 18 | 10 | 12 | 14 | 6 => {
            format_symbol_hover(&name, &info.kind_label, &info.scope)
        }
        // Reference symbols — try to resolve to definition
        1 | 3 | 17 | 19 | 9 | 11 | 13 | 15 | 7 => {
            resolve_reference_hover(state, &name, info.kind, &info.scope)
        }
        // Instance definitions / declarations
        2 => format_symbol_hover(&name, "instance", &info.scope),
        _ => None,
    }
}

/// Resolve a reference (ref kind) to its definition for hover display.
fn resolve_reference_hover(
    state: &WorkspaceState,
    name: &str,
    kind: u8,
    scope: &str,
) -> Option<Hover> {
    let snap = state.project.index.snapshot();

    // Determine the index kind to search
    let index_kind = match kind {
        1 => Some(IndexKind::Component), // ClassRef
        17 => Some(IndexKind::Enum),     // EnumRef
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

/// Human-readable label for a SymbolKind ordinal.
fn kind_label(kind: u8) -> &'static str {
    // SymbolKind ordinals from mcc
    match kind {
        0 => "component/module", // ClassDef
        1 => "→ class",          // ClassRef
        2 => "instance",         // InstDef
        3 => "→ instance",       // InstRef
        4 => "port",             // PortDef
        5 => "→ port",           // PortRef
        6 => "label",            // LabelDef
        7 => "→ label",          // LabelRef
        8 => "function",         // FuncDef
        9 => "→ function",       // FuncRef
        10 | 12 | 14 => "pin",   // Pin*Def
        11 | 13 | 15 => "→ pin", // Pin*Ref
        16 | 18 => "enum",       // EnumDef/EnumValDef
        17 | 19 => "→ enum",     // EnumRef/EnumValRef
        20 => "role",            // RoleDef
        21 => "param",           // ParamDef
        22 => "define",          // DefineDef
        23 => "attr",            // AttrDef
        24 => "→ func param",    // FuncParamRef
        25 => "bus",             // BusDef
        26 => "→ bus",           // BusRef
        27 => "unknown",         // UnknownDef
        _ => "?",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::LapperEntry;
    use crate::state::RpcSemSymbols;
    use ropey::Rope;
    use std::sync::{Arc, Mutex};
    use tower_lsp::lsp_types::{
        HoverParams, Position, TextDocumentIdentifier, TextDocumentPositionParams,
    };

    // ── Markdown formatting (pure functions) ──

    #[test]
    fn entry_line_formats_correctly() {
        let entry = IndexEntry {
            uri: Url::parse("file:///test.mc").unwrap(),
            span: (10, 20),
            name: "helper_chip".into(),
        };
        let line = format_entry_line(&entry);
        assert!(line.contains("helper_chip"), "line: {line}");
        assert!(line.contains("[10:20]"), "line: {line}");
    }

    #[test]
    fn markdown_header_and_lines() {
        let header = "### Definitions in helper.mc";
        let lines: Vec<String> = vec!["- helper_chip [0:20]".into()];
        let content = format_markdown(header, &lines);
        assert_eq!(content.kind, MarkupKind::Markdown);
        assert!(content.value.contains(header));
        assert!(content.value.contains("helper_chip"));
    }

    #[test]
    fn markdown_hover_empty_lines_returns_none() {
        let result = format_markdown_hover(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn markdown_hover_with_lines() {
        let lines = vec!["line 1".to_string(), "line 2".to_string()];
        let hover = format_markdown_hover(&lines).unwrap();
        match &hover.contents {
            HoverContents::Markup(mc) => {
                assert!(mc.value.contains("line 1"));
                assert!(mc.value.contains("line 2"));
            }
            _ => panic!("expected Markup"),
        }
    }

    #[test]
    fn symbol_hover_format() {
        let hover = format_symbol_hover("helper_chip", "component/module", "global").unwrap();
        match &hover.contents {
            HoverContents::Markup(mc) => {
                assert!(mc.value.contains("helper_chip"));
                assert!(mc.value.contains("component/module"));
            }
            _ => panic!("expected Markup"),
        }
    }

    // ── Symbol hover ──

    fn state_with_lapper(
        lapper_entries: Vec<(u8, usize, usize, u32, &str)>,
    ) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        let source = "component main                \n";
        state.insert_document(uri.clone(), Rope::from_str(source), 1);
        let lapper: Vec<LapperEntry> = lapper_entries
            .into_iter()
            .map(|(kind, start, stop, id, scope)| LapperEntry {
                kind,
                start,
                stop,
                id,
                scope: scope.into(),
                file: "file:///test.mc".into(),
            })
            .collect();
        let symbols = RpcSemSymbols {
            lapper,
            ..Default::default()
        };
        state
            .symbols
            .sem_symbols
            .insert(uri.clone(), Arc::new(Mutex::new(symbols)));
        (state, uri)
    }

    #[test]
    fn class_def_hover_shows_kind() {
        // "component main" — "main" from byte 10 to 14
        let (state, uri) = state_with_lapper(vec![(0, 10, 14, 0, "")]);
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(0, 12),
            },
            work_done_progress_params: Default::default(),
        };
        let hover = resolve(&state, &params).unwrap();
        match &hover.contents {
            HoverContents::Markup(mc) => {
                assert!(
                    mc.value.contains("component/module"),
                    "expected component/module label, got: {}",
                    mc.value
                );
            }
            _ => panic!("expected Markup"),
        }
    }

    #[test]
    fn empty_lapper_returns_none() {
        let (state, uri) = state_with_lapper(vec![]);
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(0, 0),
            },
            work_done_progress_params: Default::default(),
        };
        let result = resolve(&state, &params);
        assert!(result.is_none(), "expected None for empty lapper");
    }

    #[test]
    fn out_of_bounds_position_returns_none() {
        let (state, uri) = state_with_lapper(vec![(0, 0, 10, 0, "")]);
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(100, 0),
            },
            work_done_progress_params: Default::default(),
        };
        let result = resolve(&state, &params);
        assert!(result.is_none(), "expected None for out-of-bounds");
    }
}
