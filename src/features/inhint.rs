//! Inlay Hints — Inline hints
//!
//! LSP entry point: `textDocument/inlayHint`
//!
//! Inlay hints are inline hints displayed in code, for example:
//! - Type hints: `let x: Type = ...`
//! - Parameter names: `func(arg1: value1, arg2: value2)`
//!
//! No hints are generated currently. The previous placeholder (a `: id=N`
//! label after global declarations) was removed — it was purely a debug aid.

use crate::state::WorkspaceState;
use tower_lsp::lsp_types::{InlayHint, Range, Url};

/// Compute inlay hints. Currently returns no hints.
pub fn compute(_state: &WorkspaceState, _uri: &Url, _range: Range) -> Option<Vec<InlayHint>> {
    Some(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;
    use tower_lsp::lsp_types::Position;

    #[test]
    fn no_hints_generated() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str("component X {}\n"), 1);
        let range = Range::new(Position::new(0, 0), Position::new(0, 20));
        let hints = compute(&state, &uri, range).unwrap();
        assert!(hints.is_empty(), "expected no hints, got {hints:?}");
    }
}
