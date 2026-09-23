//! Document highlight — every occurrence of the symbol under the cursor in
//! the current file, from the same lapper + local tables `references::resolve`
//! walks. Declaration spans highlight as a write; every other hit as text.

use ropey::Rope;
use tower_lsp::lsp_types::{
    DocumentHighlight, DocumentHighlightKind, Position, Range, Url,
};

use crate::common::position::{offset_to_position, position_to_offset};
use crate::state::WorkspaceState;

pub fn resolve(
    state: &WorkspaceState,
    uri: &Url,
    position: Position,
) -> Option<Vec<DocumentHighlight>> {
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(position, &rope)?;
    let symbols_ref = state.symbols.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

    let intervals: Vec<_> = symbols
        .lapper
        .iter()
        .filter(|i| offset >= i.start && offset < i.stop)
        .collect();

    let symbol_id = intervals.first()?.id;

    let span = |rope: &Rope, a: usize, b: usize| -> Option<Range> {
        Some(Range::new(
            offset_to_position(a, rope)?,
            offset_to_position(b.min(rope.len_bytes()), rope)?,
        ))
    };

    let mut highlights = Vec::new();
    // The declaration is a write; the identifier cursor sits on it too, so
    // `is_decl` guards against the same span arriving twice.
    for decl in &symbols.local_declares {
        if decl.id == symbol_id {
            highlights.push(DocumentHighlight {
                range: span(&rope, decl.span[0], decl.span[1])?,
                kind: Some(DocumentHighlightKind::WRITE),
            });
            break;
        }
    }
    for ref_info in &symbols.local_references {
        if ref_info.declare_id == Some(symbol_id) || ref_info.id == symbol_id {
            let is_decl = symbols.local_declares.iter().any(|d| d.id == ref_info.id);
            if !is_decl {
                highlights.push(DocumentHighlight {
                    range: span(&rope, ref_info.span[0], ref_info.span[1])?,
                    kind: Some(DocumentHighlightKind::READ),
                });
            }
        }
    }
    if highlights.is_empty() {
        None
    } else {
        Some(highlights)
    }
}

/// `prepareRename` — the word range at the cursor, so the editor shows an
/// inline edit box anchored on the identifier instead of a dead-end error.
pub fn prepare(rope: &Rope, position: Position) -> Option<Range> {
    let offset = position_to_offset(position, rope)?;
    let is_word_byte = |b: Option<u8>| b.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
    let mut start = offset;
    while start > 0 && is_word_byte(rope.get_byte(start - 1)) {
        start -= 1;
    }
    let mut end = offset;
    while end < rope.len_bytes() && is_word_byte(rope.get_byte(end)) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(Range::new(
        offset_to_position(start, rope)?,
        offset_to_position(end, rope)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(text: &str) -> Rope {
        Rope::from_str(text)
    }

    #[test]
    fn prepare_covers_the_identifier() {
        let r = rope("module main {\n    RES r1;\n}\n");
        // Cursor inside `main` (byte 9).
        let range = prepare(&r, Position::new(0, 9)).unwrap();
        assert_eq!(range.start, Position::new(0, 7));
        assert_eq!(range.end, Position::new(0, 11));
        // Cursor on whitespace — no word, no rename box.
        assert_eq!(prepare(&r, Position::new(1, 2)), None);
        assert_eq!(prepare(&r, Position::new(0, 13)), None);
    }

    #[test]
    fn prepare_at_end_of_word_reaches_back() {
        let r = rope("use ./power.mc\n");
        // Cursor just past `use` (byte 3).
        let range = prepare(&r, Position::new(0, 3)).unwrap();
        assert_eq!(range.start, Position::new(0, 0));
        assert_eq!(range.end, Position::new(0, 3));
    }
}
