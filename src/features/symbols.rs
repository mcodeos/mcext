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
    /// Lapper kind as SymbolKind ordinal (u8). Maps to kind_names[] sent by mcc.
    pub kind: u8,
    /// Declare / ref ID from the lapper.
    pub id: u32,
    /// Scope string.
    pub scope: String,
    /// Human-readable label (e.g. "port", "→ instance").
    pub kind_label: String,
}

/// Priority rank for interval kinds (lower = more specific / preferred).
/// kind is a SymbolKind ordinal (u8) matching mcc's `kind_names` ordering.
pub fn kind_rank(kind: u8) -> u8 {
    // SymbolKind ordinals: ClassDef=0, ClassRef=1, InstDef=2, InstRef=3,
    // PortDef=4, PortRef=5, LabelDef=6, LabelRef=7, FuncDef=8, FuncRef=9,
    // PinIdDef=10, PinIdRef=11, PinNameDef=12, PinNameRef=13,
    // PinIfaceDef=14, PinIfaceRef=15, EnumDef=16, EnumRef=17,
    // EnumValDef=18, EnumValRef=19, RoleDef=20, ParamDef=21,
    // DefineDef=22, AttrDef=23
    match kind {
        0 | 8 | 20 => 0,        // ClassDef, FuncDef, RoleDef
        1 => 1,                 // ClassRef
        3 | 5 | 4 | 7 | 9 => 2, // InstRef, PortRef, PortDef, LabelRef, FuncRef
        10..=15 => 3,           // Pin*Def/Pin*Ref
        2 | 6 => 4,             // InstDef, LabelDef
        16..=19 => 5,           // Enum*
        21 | 22 | 23 => 6,      // ParamDef, DefineDef, AttrDef
        _ => 7,
    }
}

/// Find all lapper entries covering a position, sorted by specificity.
pub fn find_intervals_at<'a>(lapper: &'a [LapperEntry], offset: usize) -> Vec<&'a LapperEntry> {
    let mut entries: Vec<_> = lapper
        .iter()
        .filter(|e| offset >= e.start && offset <= e.stop)
        .collect();
    entries.sort_by(|a, b| kind_rank(a.kind).cmp(&kind_rank(b.kind)));
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
        kind: best.kind,
        id: best.id,
        scope: best.scope.clone(),
        kind_label: kind_label(best.kind),
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
/// kind is a SymbolKind ordinal (u8) matching mcc's `kind_names` ordering.
pub fn kind_label(kind: u8) -> String {
    match kind {
        0 => "component/module".into(), // ClassDef
        1 => "→ class".into(),          // ClassRef
        2 => "instance".into(),         // InstDef
        3 => "→ instance".into(),       // InstRef
        4 => "port".into(),             // PortDef
        5 => "→ port".into(),           // PortRef
        6 => "label".into(),            // LabelDef
        7 => "→ label".into(),          // LabelRef
        8 => "function".into(),         // FuncDef
        9 => "→ function".into(),       // FuncRef
        10 | 12 | 14 => "pin".into(),   // PinDef
        11 | 13 | 15 => "→ pin".into(), // PinRef
        16 | 18 => "enum".into(),       // EnumDef/EnumValDef
        17 | 19 => "→ enum".into(),     // EnumRef/EnumValRef
        20 => "role".into(),            // RoleDef
        21 => "param".into(),           // ParamDef
        22 => "define".into(),          // DefineDef
        23 => "attr".into(),            // AttrDef
        24 => "→ func param".into(),    // FuncParamRef
        25 => "bus".into(),             // BusDef
        26 => "→ bus".into(),           // BusRef
        27 => "unknown".into(),         // UnknownDef
        _ => "?".into(),
    }
}
