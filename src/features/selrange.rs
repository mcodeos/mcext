//! Selection range — the expandable span chain for `Expand Selection`.
//! Built from the document's lapper intervals (innermost entry containing the
//! cursor outward) with the identifier word as the innermost rung, so the
//! first expansion always picks the symbol under the cursor.

use ropey::Rope;
use tower_lsp::lsp_types::{Position, Range, SelectionRange, Url};

use crate::common::position::{offset_to_position, position_to_offset};
use crate::state::WorkspaceState;

pub fn resolve(
    state: &WorkspaceState,
    uri: &Url,
    positions: Vec<Position>,
) -> Option<Vec<SelectionRange>> {
    let rope = state.document_rope(uri)?;
    let symbols_ref = state.symbols.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;
    Some(
        positions
            .iter()
            .filter_map(|p| one(&rope, &symbols.lapper, *p))
            .collect(),
    )
}

/// The span chain for one cursor position. Rungs strictly grow: the word rung
/// first, then every lapper span containing the cursor, smallest first. The
/// chain is built in byte spans and converted to `Position`s once at the end.
fn one(
    rope: &Rope,
    lapper: &[crate::rpc::LapperEntry],
    position: Position,
) -> Option<SelectionRange> {
    let offset = position_to_offset(position, rope)?;

    let to_range = |span: (usize, usize)| -> Option<Range> {
        Some(Range::new(
            offset_to_position(span.0, rope)?,
            offset_to_position(span.1.min(rope.len_bytes()), rope)?,
        ))
    };

    let mut chain: Vec<(usize, usize)> = Vec::new();
    if let Some(word) = word_span(rope, offset) {
        chain.push(word);
    }
    // Lapper entries overlap freely (a reference span can sit inside a scope
    // span); sort candidates smallest-first and keep only strictly growing
    // ones so `parent` chains never collapse a rung.
    let mut containing: Vec<(usize, usize)> = lapper
        .iter()
        .filter(|e| offset >= e.start && offset < e.stop && e.start < e.stop)
        .map(|e| (e.start, e.stop))
        .collect();
    containing.sort_by_key(|(a, b)| (b - a, *a));
    for span in containing {
        // An enclosing rung may share the start (a scope span wrapping the
        // word) but must strictly extend the end — this also drops exact
        // duplicates.
        let grows = chain
            .last()
            .map(|last| span.0 <= last.0 && span.1 > last.1)
            .unwrap_or(true);
        if grows {
            chain.push(span);
        }
    }

    // Fold the chain into the parent-linked list, outermost first.
    let mut node: Option<SelectionRange> = None;
    for span in chain.into_iter().rev() {
        node = Some(SelectionRange {
            range: to_range(span)?,
            parent: node.map(Box::new),
        });
    }
    node
}

/// The identifier word around `offset` as a byte span (`None` on whitespace).
fn word_span(rope: &Rope, offset: usize) -> Option<(usize, usize)> {
    let is_word_byte = |b: Option<u8>| b.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
    let mut start = offset;
    while start > 0 && is_word_byte(rope.get_byte(start - 1)) {
        start -= 1;
    }
    let mut end = offset;
    while end < rope.len_bytes() && is_word_byte(rope.get_byte(end)) {
        end += 1;
    }
    (start != end).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::LapperEntry;

    fn lapper(entries: &[(usize, usize)]) -> Vec<LapperEntry> {
        entries
            .iter()
            .enumerate()
            .map(|(i, (s, e))| LapperEntry {
                kind: 0,
                start: *s,
                stop: *e,
                id: i as u32,
                scope: String::new(),
                file: String::new(),
            })
            .collect()
    }

    fn pos(rope: &Rope, off: usize) -> Position {
        let line = rope.byte_to_line(off);
        Position::new(line as u32, (off - rope.line_to_byte(line)) as u32)
    }

    fn chain(rope: &Rope, entries: &[(usize, usize)], off: usize) -> Vec<(usize, usize)> {
        let node = one(rope, &lapper(entries), pos(rope, off)).expect("chain");
        let mut rungs = Vec::new();
        let mut cur = Some(&node);
        while let Some(n) = cur {
            rungs.push((
                rope.line_to_byte(n.range.start.line as usize) + n.range.start.character as usize,
                rope.line_to_byte(n.range.end.line as usize) + n.range.end.character as usize,
            ));
            cur = n.parent.as_deref();
        }
        rungs
    }

    #[test]
    fn word_first_then_enclosing_spans() {
        let r = Rope::from_str("module main { let cap = 1; }\n");
        // Cursor inside `cap` (bytes 18..21).
        let rungs = chain(&r, &[(14, 26), (12, 27)], 19);
        assert_eq!(rungs, vec![(18, 21), (14, 26), (12, 27)]);
    }

    #[test]
    fn duplicate_spans_collapse_to_one_rung() {
        let r = Rope::from_str("module main { let cap = 1; }\n");
        let rungs = chain(&r, &[(14, 26), (14, 26)], 19);
        assert_eq!(rungs, vec![(18, 21), (14, 26)]);
    }

    #[test]
    fn cursor_on_whitespace_gets_no_chain() {
        let r = Rope::from_str("   \n");
        assert!(one(&r, &lapper(&[]), pos(&r, 1)).is_none());
    }
}
