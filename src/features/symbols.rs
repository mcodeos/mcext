//! Shared symbol resolution for F12 / Hover / Completion.
//!
//! Provides a single entry point for finding a symbol at a cursor position
//! and resolving it to its definition, shared across all IDE features.

use crate::rpc::LapperEntry;
use crate::state::WorkspaceState;
use tower_lsp::lsp_types::{Position, Url};

/// Information about a resolved symbol.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    /// The URI of the file containing this symbol.
    pub uri: Url,
    /// Byte range in the source file.
    pub span: (usize, usize),
    /// Lapper kind string (e.g. "instance_ref", "port_def").
    pub kind: String,
    /// Declare / ref ID from the lapper.
    pub id: u32,
    /// Scope string.
    pub scope: String,
    /// Human-readable label (e.g. "port", "→ instance").
    pub kind_label: String,
}

/// Priority rank for interval kinds (lower = more specific / preferred).
/// Mirrors the sort order in gotodef.
pub fn kind_rank(kind: &str) -> u8 {
    match kind {
        "class_def" | "function_def" | "role_def" => 0,
        "class_ref" | "declare_class" => 1,
        "instance_ref" | "port_def" | "label_ref" => 2,
        "pin_name_def" | "pin_name_ref" => 3,
        "instance_def" | "declare_instance" | "label_def" => 4,
        "enum_value_def" | "enum_value_ref" | "enum_class_ref" | "enum_class_def" => 5,
        "function_ref" | "interface_ref" | "define_def" => 6,
        _ => 7,
    }
}

/// Find all lapper entries covering a position, sorted by specificity.
pub fn find_intervals_at<'a>(lapper: &'a [LapperEntry], offset: usize) -> Vec<&'a LapperEntry> {
    let mut entries: Vec<_> = lapper
        .iter()
        .filter(|e| offset >= e.start && offset <= e.stop)
        .collect();
    entries.sort_by(|a, b| kind_rank(&a.kind).cmp(&kind_rank(&b.kind)));
    entries
}

/// Find the most specific lapper entry at a byte offset.
/// Returns the best entry and its source text.
pub fn find_symbol_at_offset(
    state: &WorkspaceState,
    uri: &Url,
    offset: usize,
) -> Option<(SymbolInfo, String)> {
    let rope = state.document_rope(uri)?;
    let symbols_ref = state.symbols.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

    let intervals = find_intervals_at(&symbols.lapper, offset);
    let best = intervals.first()?;

    let name = rope.byte_slice(best.start..best.stop).to_string();
    let info = SymbolInfo {
        uri: uri.clone(),
        span: (best.start, best.stop),
        kind: best.kind.clone(),
        id: best.id,
        scope: best.scope.clone(),
        kind_label: kind_label(&best.kind),
    };

    Some((info, name))
}

/// Find the most specific lapper entry at a cursor position.
/// Convenience wrapper around `find_symbol_at_offset`.
pub fn find_symbol_at_cursor(
    state: &WorkspaceState,
    uri: &Url,
    position: Position,
) -> Option<(SymbolInfo, String)> {
    let rope = state.document_rope(uri)?;
    let offset = crate::common::position::position_to_offset(position, &rope)?;
    find_symbol_at_offset(state, uri, offset)
}

/// Human-readable label for a lapper kind.
pub fn kind_label(kind: &str) -> String {
    match kind {
        "class_def" | "class_definition" => "component/module".into(),
        "class_ref" | "declare_class" => "→ class".into(),
        "port_def" => "port".into(),
        "label_def" => "label".into(),
        "label_ref" => "→ label".into(),
        "function_def" => "function".into(),
        "function_ref" => "→ function".into(),
        "pin_name_def" => "pin".into(),
        "pin_name_ref" => "→ pin".into(),
        "enum_class_def" | "enum_value_def" => "enum".into(),
        "enum_class_ref" | "enum_value_ref" => "→ enum".into(),
        "instance_def" | "declare_instance" => "instance".into(),
        "instance_ref" => "→ instance".into(),
        "define_def" => "define".into(),
        "role_def" => "role".into(),
        "interface_ref" => "→ interface".into(),
        _ => kind.into(),
    }
}
