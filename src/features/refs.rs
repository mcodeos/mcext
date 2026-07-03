//! Find References — Find references
//!
//! LSP entry point: `textDocument/references`
//! Data source: RpcSemSymbols from sem RPC
//!
//! Phase 0 only in-file references; cross-file references need project index.

use crate::common::position::position_to_offset;
use crate::state::WorkspaceState;
use tower_lsp::lsp_types::{Location, Position, Range, Url};

/// Compute references response.
///
/// `include_declaration` controls whether to include the declaration location itself.
pub fn resolve(
    state: &WorkspaceState,
    uri: &Url,
    position: Position,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(position, &rope)?;
    let symbols_ref = state.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

    // Find symbol at cursor position
    let intervals: Vec<_> = symbols
        .lapper
        .iter()
        .filter(|i| offset >= i.start && offset < i.stop)
        .collect();

    if intervals.is_empty() {
        return None;
    }

    let mut locations = Vec::new();

    // Get the symbol id at cursor
    let symbol_id = intervals.first()?.id;

    // Find declarations
    for decl in &symbols.local_declares {
        if decl.id == symbol_id {
            let start = offset_to_position(decl.span[0], &rope)?;
            let end = offset_to_position(decl.span[1], &rope)?;
            locations.push(Location::new(uri.clone(), Range::new(start, end)));
            break;
        }
    }

    // Find references (including or excluding declaration)
    for ref_info in &symbols.local_references {
        if ref_info.declare_id == Some(symbol_id) || ref_info.id == symbol_id {
            let is_decl = symbols.local_declares.iter().any(|d| d.id == ref_info.id);
            if !is_decl || include_declaration {
                let start = offset_to_position(ref_info.span[0], &rope)?;
                let end = offset_to_position(ref_info.span[1], &rope)?;
                locations.push(Location::new(uri.clone(), Range::new(start, end)));
            }
        }
    }

    if locations.is_empty() {
        None
    } else {
        Some(locations)
    }
}

/// Helper: convert offset to Position
fn offset_to_position(offset: usize, rope: &ropey::Rope) -> Option<Position> {
    let line = rope.try_byte_to_line(offset).ok()?;
    let line_start = rope.try_line_to_char(line).ok()?;
    let col = offset - line_start;
    Some(Position::new(line as u32, col as u32))
}
