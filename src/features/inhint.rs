//! Inlay Hints — Inline hints
//!
//! LSP entry point: `textDocument/inlayHint`
//!
//! Inlay hints are inline hints displayed in code, for example:
//! - Type hints: `let x: Type = ...`
//! - Parameter names: `func(arg1: value1, arg2: value2)`
//!
//! Currently implemented is simple type hints, showing instantiation type of component/interface.

use crate::common::position::{offset_to_position, position_to_offset};
use crate::state::WorkspaceState;
use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Range, Url};

/// Compute inlay hints
pub fn compute(state: &WorkspaceState, uri: &Url, range: Range) -> Option<Vec<InlayHint>> {
    let rope = state.document_rope(uri)?;
    let symbols_ref = state.symbols.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

    // If no symbols, return None (consistent with other feature modules)
    if symbols.lapper.is_empty() && symbols.global_declares.is_empty() {
        return None;
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
        let h_offset = match position_to_offset(h.position, &rope) {
            Some(o) => o,
            None => return false, // drop hints at unrepresentable positions
        };
        h_offset >= range_start && h_offset <= range_end
    });

    Some(hints)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RpcSemSymbols;
    use ropey::Rope;
    use std::sync::{Arc, Mutex};
    use tower_lsp::lsp_types::Position;

    fn make_state(source: &str, declares: Vec<(u32, usize, usize)>) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str(source), 1);

        let global_declares: Vec<_> = declares
            .into_iter()
            .map(|(id, start, end)| crate::state::GlobalDeclareSpan {
                id,
                uri: "/test.mc".into(),
                span: [start, end],
            })
            .collect();

        let symbols = RpcSemSymbols {
            lapper: vec![],
            local_declares: vec![],
            local_references: vec![],
            global_declares,
            global_references: vec![],
            cross_file_targets: vec![],
        };
        state
            .symbols
            .sem_symbols
            .insert(uri.clone(), Arc::new(Mutex::new(symbols)));
        (state, uri)
    }

    #[test]
    fn no_symbols_returns_none() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str("x=1\n"), 1);
        let range = Range::new(Position::new(0, 0), Position::new(0, 5));
        let result = compute(&state, &uri, range);
        assert!(result.is_none(), "expected None, got {result:?}");
    }

    #[test]
    fn global_declare_generates_hint() {
        let (state, uri) = make_state("component X {}\n", vec![(42, 0, 12)]);
        let range = Range::new(Position::new(0, 0), Position::new(0, 20));
        let hints = compute(&state, &uri, range).unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints.len(), 1);
        match &hints[0].label {
            InlayHintLabel::String(s) => assert!(s.contains("id=42"), "expected id=42, got {s}"),
            _ => panic!("expected String label"),
        }
    }

    #[test]
    fn hint_filtered_by_range() {
        let (state, uri) = make_state("aaa bbb\nccc ddd\n", vec![(1, 10, 13)]);
        // Range only covers first line (bytes 0..4), declare at byte 10 should be excluded
        let range = Range::new(Position::new(0, 0), Position::new(0, 4));
        let hints = compute(&state, &uri, range).unwrap();
        assert!(
            hints.is_empty(),
            "hint at byte 10 should be excluded from range 0..4"
        );
    }

    #[test]
    fn hint_included_in_range() {
        let (state, uri) = make_state("abc\n", vec![(1, 0, 3)]);
        let range = Range::new(Position::new(0, 0), Position::new(0, 5));
        let hints = compute(&state, &uri, range).unwrap();
        assert_eq!(hints.len(), 1);
    }
}
