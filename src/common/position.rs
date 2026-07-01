//! LSP `Position` ↔ Rope byte offset conversion
//!
//! mcc uses byte offset; LSP uses `(line, character)` (UTF-16 code units).
//! When VSCode defaults `files.encoding = utf8`, character equals UTF-8 byte offset;
//! By default `vscode-languageclient` already handles correctly, here we assume `character == byte`.

use ropey::Rope;
use tower_lsp::lsp_types::Position;

/// Byte offset → LSP Position.
///
/// Returns `None` on out-of-bounds, doesn't panic.
pub fn offset_to_position(byte_offset: usize, rope: &Rope) -> Option<Position> {
    // Convert byte offset to char offset
    let rope_len = rope.len_bytes();
    if byte_offset > rope_len {
        return None;
    }
    let char_offset = rope.try_byte_to_char(byte_offset).ok()?;
    let line = rope.try_char_to_line(char_offset).ok()?;
    let first_char_of_line = rope.try_line_to_char(line).ok()?;
    let column = char_offset - first_char_of_line;
    Some(Position::new(line as u32, column as u32))
}

/// LSP Position → byte offset.
///
/// Returns **document-level** offset (not in-line offset).
///
/// Returns `None` on out-of-bounds. When `character` exceeds line length, clamps to line end (no panic).
pub fn position_to_offset(position: Position, rope: &Rope) -> Option<usize> {
    let line = position.line as usize;
    if line >= rope.len_lines() {
        return None;
    }
    let line_char_offset = rope.try_line_to_char(line).ok()?;
    // Last line's line + 1 doesn't exist, use total character count as fallback
    let line_end = rope
        .try_line_to_char(line + 1)
        .ok()
        .unwrap_or_else(|| rope.len_chars());
    let line_content_len = line_end.saturating_sub(line_char_offset);
    let col = (position.character as usize).min(line_content_len);
    let target_offset = line_char_offset + col;
    
    // Clamp target_offset to rope char length to prevent panic
    let rope_len_chars = rope.len_chars();
    if target_offset > rope_len_chars {
        return None;
    }
    
    // Use document-level slice to get absolute offset
    // Clamp to rope boundaries to prevent panic
    let slice_end = target_offset.min(rope_len_chars);
    let slice = rope.get_slice(0..slice_end)?;
    Some(slice.len_bytes())
}

/// Byte offset → line number (1-based, line number consistent with mcc::Location.row)
pub fn offset_to_line(byte_offset: usize, rope: &Rope) -> Option<u32> {
    let char_offset = rope.try_byte_to_char(byte_offset).ok()?;
    let line = rope.try_char_to_line(char_offset).ok()?;
    Some(line as u32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    #[test]
    fn roundtrip_single_line() {
        let rope = Rope::from_str("hello world");
        let pos = offset_to_position(6, &rope).unwrap();
        assert_eq!(pos, Position::new(0, 6));
        let offset = position_to_offset(pos, &rope).unwrap();
        assert_eq!(offset, 6);
    }

    #[test]
    fn roundtrip_multiline() {
        let rope = Rope::from_str("abc\ndef\nghi");
        // 'd' is on line 1 (0-based), column 0
        let pos = offset_to_position(4, &rope).unwrap();
        assert_eq!(pos, Position::new(1, 0));
        // 2nd line, 2nd character 'i': offset 10
        let pos = offset_to_position(10, &rope).unwrap();
        assert_eq!(pos, Position::new(2, 2));
        let back = position_to_offset(pos, &rope).unwrap();
        assert_eq!(back, 10);
    }

    #[test]
    fn out_of_bounds_returns_none() {
        let rope = Rope::from_str("abc");
        assert!(offset_to_position(100, &rope).is_none());
        let pos = Position::new(99, 0);
        assert!(position_to_offset(pos, &rope).is_none());
    }
}
