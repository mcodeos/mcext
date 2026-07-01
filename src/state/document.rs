//! State of a single open document + Rope incremental editing

use crate::common::position::position_to_offset;
use ropey::Rope;
use tower_lsp::lsp_types::{Position, TextDocumentContentChangeEvent};

/// LSP document version number (corresponds to VSCode's `TextDocumentItem.version`)
pub type DocumentVersion = i32;

/// Document entry: Rope buffer + version
#[derive(Debug, Clone)]
pub struct DocumentEntry {
    pub rope: Rope,
    pub version: DocumentVersion,
}

/// Apply a set of incremental changes to Rope
///
/// Supports two modes (determined by `change.range`):
/// - `Some(range)` —— INCREMENTAL mode: delete range, insert text
/// - `None` —— FULL mode: replace entire with text
///
/// Multiple changes applied in order; subsequent range coordinates based on previous result.
///
/// Returns error on out-of-bounds, doesn't panic.
pub fn apply_changes(
    rope: &mut Rope,
    changes: &[TextDocumentContentChangeEvent],
) -> Result<(), String> {
    for change in changes {
        match change.range {
            Some(range) => {
                let start_byte = position_to_offset(range.start, rope)
                    .ok_or_else(|| format!("invalid range start: {:?}", range.start))?;
                let end_byte = position_to_offset(range.end, rope)
                    .ok_or_else(|| format!("invalid range end: {:?}", range.end))?;

                let rope_len = rope.len_bytes();
                if start_byte > rope_len || end_byte > rope_len || start_byte > end_byte {
                    return Err(format!(
                        "invalid range [{}, {}) for rope len {}",
                        start_byte, end_byte, rope_len
                    ));
                }

                // Convert byte offsets to char offsets (ropey remove/insert use char indices)
                let start_char = rope
                    .try_byte_to_char(start_byte)
                    .map_err(|e| format!("invalid byte offset {}: {}", start_byte, e))?;
                let end_char = rope
                    .try_byte_to_char(end_byte)
                    .map_err(|e| format!("invalid byte offset {}: {}", end_byte, e))?;

                let rope_chars = rope.len_chars();
                if start_char > rope_chars || end_char > rope_chars || start_char > end_char {
                    return Err(format!(
                        "invalid char range [{}, {}) for rope char len {}",
                        start_char, end_char, rope_chars
                    ));
                }

                rope.remove(start_char..end_char);
                if !change.text.is_empty() {
                    rope.insert(start_char, &change.text);
                }
            }
            None => {
                // FULL: replace entire
                *rope = Rope::from_str(&change.text);
            }
        }
    }
    Ok(())
}

/// "Rehearse" a set of changes: returns version number change after application
///
/// In INCREMENTAL mode, each change increments version by +1.
pub fn next_version(current: DocumentVersion, is_incremental: bool) -> DocumentVersion {
    if is_incremental {
        current + 1
    } else {
        current
    }
}

/// Compute LSP Position → byte offset (with error message)
pub fn pos_to_offset_or_err(pos: Position, rope: &Rope) -> Result<usize, String> {
    position_to_offset(pos, rope).ok_or_else(|| format!("invalid position: {pos:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Range;

    fn change(range: Option<Range>, text: &str) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range,
            range_length: None,
            text: text.to_string(),
        }
    }

    fn range(start: u32, end_line: u32, end_col: u32) -> Range {
        Range::new(Position::new(0, start), Position::new(end_line, end_col))
    }

    #[test]
    fn full_replace() {
        let mut rope = Rope::from_str("hello");
        apply_changes(&mut rope, &[change(None, "world")]).unwrap();
        assert_eq!(rope.to_string(), "world");
    }

    #[test]
    fn incremental_insert() {
        let mut rope = Rope::from_str("hello world");
        // Insert at offset 6 (space position) ', '
        apply_changes(&mut rope, &[change(Some(range(6, 0, 6)), ", ")]).unwrap();
        assert_eq!(rope.to_string(), "hello , world");
    }

    #[test]
    fn incremental_replace() {
        let mut rope = Rope::from_str("hello world");
        // Replace "world" → "rust"
        apply_changes(&mut rope, &[change(Some(range(6, 0, 11)), "rust")]).unwrap();
        assert_eq!(rope.to_string(), "hello rust");
    }

    #[test]
    fn incremental_delete() {
        let mut rope = Rope::from_str("hello world");
        apply_changes(&mut rope, &[change(Some(range(5, 0, 11)), "")]).unwrap();
        assert_eq!(rope.to_string(), "hello");
    }

    #[test]
    fn multiple_changes_sequential() {
        let mut rope = Rope::from_str("abc");
        // First insert "X" at offset 1, then insert "Y" at offset 2
        apply_changes(
            &mut rope,
            &[
                change(Some(range(1, 0, 1)), "X"),
                change(Some(range(2, 0, 2)), "Y"),
            ],
        )
        .unwrap();
        assert_eq!(rope.to_string(), "aXYbc");
    }

    #[test]
    fn invalid_range_returns_error() {
        let mut rope = Rope::from_str("hi");
        // range exceeds rope bounds
        let result = apply_changes(&mut rope, &[change(Some(range(0, 5, 5)), "x")]);
        assert!(result.is_err());
    }

    #[test]
    fn next_version_incremental() {
        assert_eq!(next_version(1, true), 2);
        assert_eq!(next_version(1, false), 1);
    }
}
