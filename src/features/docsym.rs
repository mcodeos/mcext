//! Document Symbols (§15.3) — file outline from lapper def entries.
//!
//! Iterates all DEF-type lapper entries and builds a DocumentSymbol tree
//! grouped by container (scope). Uses SymbolKind ordinals from mcc.

use crate::rpc::LapperEntry;
use tower_lsp::lsp_types::{DocumentSymbol, Position, Range, SymbolKind, Url};

/// Build document symbols from lapper entries.
/// Only DEF kinds are included (not refs).
pub fn document_symbols(
    lapper: &[LapperEntry],
    uri: &Url,
    rope: &ropey::Rope,
) -> Vec<DocumentSymbol> {
    let mut symbols: Vec<DocumentSymbol> = Vec::new();

    for entry in lapper {
        // Only include definition kinds
        let (name, kind) = match def_symbol_info(entry.kind) {
            Some(info) => info,
            None => continue,
        };

        let name_str = rope.byte_slice(entry.start..entry.stop).to_string();

        let range = Range {
            start: offset_to_position(entry.start, rope),
            end: offset_to_position(entry.stop, rope),
        };

        symbols.push(DocumentSymbol {
            name: name_str,
            detail: Some(name.to_string()),
            kind,
            tags: None,
            deprecated: None,
            range,
            selection_range: range,
            children: None,
        });
    }

    symbols
}

/// Map SymbolKind ordinal to LSP SymbolKind + display name.
fn def_symbol_info(kind: u8) -> Option<(&'static str, SymbolKind)> {
    match kind {
        0 => Some(("class", SymbolKind::CLASS)),       // ClassDef
        2 => Some(("instance", SymbolKind::VARIABLE)), // InstDef
        4 => Some(("port", SymbolKind::PROPERTY)),     // PortDef
        6 => Some(("label", SymbolKind::STRING)),      // LabelDef
        8 => Some(("function", SymbolKind::FUNCTION)), // FuncDef
        10 | 12 | 14 => Some(("pin", SymbolKind::ENUM_MEMBER)), // Pin*Def
        16 => Some(("enum", SymbolKind::ENUM)),        // EnumDef
        18 => Some(("enum value", SymbolKind::ENUM_MEMBER)), // EnumValDef
        20 => Some(("role", SymbolKind::INTERFACE)),   // RoleDef
        21 => Some(("param", SymbolKind::TYPE_PARAMETER)), // ParamDef
        22 => Some(("define", SymbolKind::CONSTANT)),  // DefineDef
        23 => Some(("attr", SymbolKind::KEY)),         // AttrDef
        25 => Some(("bus", SymbolKind::PROPERTY)),     // BusDef
        27 => Some(("unknown", SymbolKind::VARIABLE)), // UnknownDef
        _ => None,
    }
}

fn offset_to_position(offset: usize, rope: &ropey::Rope) -> Position {
    let line = rope.try_byte_to_line(offset).unwrap_or(0);
    let line_start = rope.try_line_to_byte(line).unwrap_or(0);
    let col = offset.saturating_sub(line_start);
    Position {
        line: line as u32,
        character: col as u32,
    }
}
