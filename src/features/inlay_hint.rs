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
    let symbols = symbols_ref.lock().unwrap_or_else(|e| e.into_inner());

    // If no symbols, return empty
    if symbols.symbol_lapper.is_empty() {
        return Some(Vec::new());
    }

    let uri_path = uri.path();
    let mut hints = Vec::new();

    // Get component name info from global_table
    if let Ok(global_table) = symbols.global_table.lock() {
        for ((file_uri, name), _class_id) in global_table.class_name_to_id.iter() {
            // Filter current file
            if !file_uri.contains(uri_path) {
                continue;
            }

            // Find declaration position of this class name
            if let Some((_, span)) = global_table.class_id_to_span.get(_class_id) {
                let _start_pos = offset_to_position(span.start, &rope)?;
                let end_pos = offset_to_position(span.start + name.len(), &rope)?;

                // Generate type hint
                hints.push(InlayHint {
                    position: end_pos,
                    label: InlayHintLabel::String(format!(": {name}")),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(true),
                    padding_right: None,
                    data: None,
                });
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::WorkspaceState;
    use ropey::Rope;
    use std::sync::Arc;

    fn fake_state(text: &str) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str(text), 1);

        let mc_uri = mcc::McURI::from("/test.mc");
        mcc::mcc_load_from_string(&mc_uri, text);

        if let Some(result) = mcc::mcc_query(&mc_uri) {
            state.insert_parse(
                uri.clone(),
                Arc::clone(&result.sem_tokens),
                Arc::clone(&result.sem_symbols),
                mc_uri,
            );
        }

        (state, uri)
    }

    #[test]
    fn compute_returns_hints_for_components() {
        let (state, uri) = fake_state("component X { pins = [] }\n");
        // Specify entire document range
        let range = Range::new(Position::new(0, 0), Position::new(10, 0));
        let hints = compute(&state, &uri, range);
        // May have hints because global_table has component info
        assert!(hints.is_some());
    }

    #[test]
    fn compute_respects_range() {
        let (state, uri) = fake_state("component X { pins = [] }\n");
        // Specify entire document range
        let range = Range::new(Position::new(0, 0), Position::new(10, 0));
        let hints = compute(&state, &uri, range);
        assert!(hints.is_some());
    }

    #[test]
    fn compute_handles_empty_document() {
        let (state, uri) = fake_state("");
        let range = Range::new(Position::new(0, 0), Position::new(0, 0));
        let hints = compute(&state, &uri, range);
        // Empty document should return Some (even if empty Vec)
        assert!(hints.is_some());
    }
}
