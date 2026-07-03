//! Inlay Hints — Inline hints
//!
//! LSP entry point: `textDocument/inlayHint`
//!
//! Inlay hints are inline hints displayed in code, for example:
//! - Type hints: `let x: Type = ...`
//! - Parameter names: `func(arg1: value1, arg2: value2)`
//!
//! Currently implemented is simple type hints, showing instantiation type of component/interface.

use crate::common::position::position_to_offset;
use crate::state::WorkspaceState;
use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Position, Range, Url};

/// Compute inlay hints
pub fn compute(state: &WorkspaceState, uri: &Url, range: Range) -> Option<Vec<InlayHint>> {
    let rope = state.document_rope(uri)?;
    let symbols_ref = state.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

    // If no symbols, return empty
    if symbols.lapper.is_empty() && symbols.global_declares.is_empty() {
        return Some(Vec::new());
    }

    let uri_path = uri.path();
    let mut hints = Vec::new();

    // Generate type hints from global_declares
    for decl in &symbols.global_declares {
        // Filter current file
        if !decl.uri.contains(uri_path) {
            continue;
        }

        // Use declaration span for positioning
        let end_offset = decl.span[1];

        let end_pos = offset_to_position(end_offset, &rope)?;

        // Generate type hint (using id as placeholder)
        hints.push(InlayHint {
            position: end_pos,
            label: InlayHintLabel::String(format!(": id={}", decl.id)),
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: None,
            padding_left: Some(true),
            padding_right: None,
            data: None,
        });
    }

    // Filter hints within range
    let range_start = position_to_offset(range.start, &rope)?;
    let range_end = position_to_offset(range.end, &rope)?;
    hints.retain(|h| {
        let h_offset = position_to_offset(h.position, &rope).unwrap_or(0);
        h_offset >= range_start && h_offset <= range_end
    });

    Some(hints)
}

/// Helper function: convert offset to position
fn offset_to_position(offset: usize, rope: &ropey::Rope) -> Option<Position> {
    let line = rope.try_byte_to_line(offset).ok()?;
    let line_start = rope.try_line_to_char(line).ok()?;
    let col = offset - line_start;
    Some(Position::new(line as u32, col as u32))
}
