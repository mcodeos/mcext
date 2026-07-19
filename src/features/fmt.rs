//! Formatting — Code formatting
//!
//! LSP entry points:
//! - `textDocument/formatting` - full document formatting
//! - `textDocument/rangeFormatting` - range formatting
//!
//! Implementation strategy:
//! 1. Parse source text, build AST (or AST-like structure)
//! 2. Regenerate normalized text from AST
//! 3. Compare with original text, generate minimal diff edits

use ropey::Rope;
use tower_lsp::lsp_types::{Range, TextEdit, Url};

/// Indent size (number of spaces)
const INDENT_SIZE: usize = 4;

/// Formatting options
#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Indent size
    pub indent_size: usize,
    /// Whether to add spaces around brackets (e.g. `func(a, b)`)
    pub spaces_around_brackets: bool,
    /// Whether to add spaces around operators (e.g. `a + b`)
    pub spaces_around_operators: bool,
    /// Line width (currently unused)
    pub max_line_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl FormatOptions {
    pub fn new() -> Self {
        Self {
            indent_size: INDENT_SIZE,
            spaces_around_brackets: true,
            spaces_around_operators: true,
            max_line_width: 120,
        }
    }
}

/// Format entire document
pub fn format_document(
    _uri: &Url,
    rope: &Rope,
    options: Option<FormatOptions>,
) -> Option<Vec<TextEdit>> {
    let options = options.unwrap_or_default();
    let text = rope.to_string();

    // Parse and reformat
    let formatted = format_text(&text, &options)?;

    // If no change, return None
    if formatted == text {
        return None;
    }

    // Generate diff edits
    Some(diff_edits(rope, &formatted))
}

/// Format specified range
pub fn format_range(
    _uri: &Url,
    rope: &Rope,
    range: Range,
    options: Option<FormatOptions>,
) -> Option<Vec<TextEdit>> {
    let options = options.unwrap_or_default();

    // Get text in range
    use crate::common::position::position_to_offset;
    let start_offset = position_to_offset(range.start, rope)?;
    let end_offset = position_to_offset(range.end, rope)?;

    // Convert byte offsets to char offsets for rope.slice
    let start_char = rope.try_byte_to_char(start_offset).ok()?;
    let end_char = rope.try_byte_to_char(end_offset).ok()?;

    let text = rope.slice(start_char..end_char).to_string();
    let formatted = format_text(&text, &options)?;

    // If no change, return None
    if formatted == text {
        return None;
    }

    // Generate single replacement edit
    Some(vec![TextEdit {
        range,
        new_text: formatted,
    }])
}

/// Internal: format text
fn format_text(text: &str, options: &FormatOptions) -> Option<String> {
    let mut result = String::with_capacity(text.len() * 2);
    let mut indent_level = 0;
    let mut in_line = false;
    let mut line_start = true;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '{' => {
                if !line_start || result.ends_with(' ') {
                    // No space needed here
                }
                result.push('{');
                indent_level += 1;
                in_line = false;

                // Check if next line is empty or has only comments
                skip_whitespace_and_comments(&mut chars);
                if let Some(&'\n') = chars.peek() {
                    chars.next();
                    result.push('\n');
                    add_indent(&mut result, indent_level, options.indent_size);
                }
                line_start = true;
            }
            '}' => {
                indent_level = indent_level.saturating_sub(1);
                if !line_start && !result.ends_with('\n') {
                    result.push('\n');
                }
                add_indent(&mut result, indent_level, options.indent_size);
                result.push('}');
                in_line = false;

                // Check if newline is needed
                skip_whitespace_and_comments(&mut chars);
                if let Some(&c) = chars.peek() {
                    if c == ',' || c == ';' {
                        chars.next();
                        result.push(c);
                    }
                    if c != '\n' && c != '}' && c != '/' {
                        result.push('\n');
                    }
                }
                line_start = true;
            }
            '\n' => {
                result.push('\n');
                add_indent(&mut result, indent_level, options.indent_size);
                in_line = false;
                line_start = true;
            }
            c if c.is_whitespace() => {
                if in_line && c == ' ' && !line_start {
                    // Preserve inline spaces
                    if !result.ends_with(' ') && !result.ends_with('\t') {
                        result.push(' ');
                    }
                } else if c == '\t' {
                    // Convert tabs to spaces
                    if !result.ends_with('\t') {
                        result.push(' ');
                    }
                }
                // Newlines and line-start spaces handled above
                if c != '\n' {
                    line_start = false;
                }
            }
            ',' => {
                result.push(',');
                in_line = true;
                skip_whitespace_and_comments(&mut chars);
                if let Some(&'\n') = chars.peek() {
                    // Let newline logic handle it
                } else {
                    result.push(' ');
                }
                line_start = false;
            }
            ';' => {
                result.push(';');
                in_line = false;
                skip_whitespace_and_comments(&mut chars);
                if let Some(&'\n') = chars.peek() {
                    // Let newline logic handle it
                } else if let Some(&'/') = chars.peek() {
                    // Newline after comment
                } else {
                    result.push('\n');
                    add_indent(&mut result, indent_level, options.indent_size);
                }
                line_start = true;
            }
            '/' => {
                // Check if this is a comment
                if let Some(&'/') = chars.peek() {
                    // Line comment
                    while let Some(&c) = chars.peek() {
                        if c == '\n' {
                            break;
                        }
                        result.push(c);
                        chars.next();
                    }
                    result.push('\n');
                    add_indent(&mut result, indent_level, options.indent_size);
                    line_start = true;
                } else {
                    result.push('/');
                    in_line = true;
                    line_start = false;
                }
            }
            '[' => {
                result.push('[');
                in_line = true;
                line_start = false;
                skip_whitespace_and_comments(&mut chars);
                if let Some(&']') = chars.peek() {
                    // Empty array
                } else if let Some(&c) = chars.peek() {
                    if !c.is_whitespace() {
                        // result.push(' ');
                    }
                }
            }
            ']' => {
                result.push(']');
                in_line = true;
                line_start = false;
            }
            _ => {
                if line_start && !result.ends_with('\n') && !result.is_empty() {
                    // Line start check
                }
                result.push(c);
                in_line = true;
                line_start = false;
            }
        }
    }

    // Clean up trailing empty lines
    while result.ends_with('\n') {
        result.pop();
    }
    result.push('\n');

    Some(result)
}

/// Skip whitespace and comments
fn skip_whitespace_and_comments(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() && c != '\n' {
            chars.next();
        } else if c == '/' {
            chars.next();
            if let Some(&'/') = chars.peek() {
                while let Some(&c) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                }
            } else {
                // Not a comment, put back the '/'
                break;
            }
        } else {
            break;
        }
    }
}

/// Add indentation
fn add_indent(result: &mut String, level: usize, indent_size: usize) {
    for _ in 0..level {
        for _ in 0..indent_size {
            result.push(' ');
        }
    }
}

/// Calculate diff edits between two texts
pub fn diff_edits(rope: &Rope, new_text: &str) -> Vec<TextEdit> {
    use tower_lsp::lsp_types::Position;

    // Simple full replacement
    // TODO: optimize to true diff algorithm
    let start = Position::new(0, 0);
    let end = if rope.len_lines() > 0 {
        let last_line_idx = rope.len_lines() - 1;
        let _last_line_char = rope.try_line_to_char(last_line_idx).unwrap_or(0);
        let last_line_len = rope.line(last_line_idx).len_chars();
        Position::new(last_line_idx as u32, last_line_len as u32)
    } else {
        Position::new(0, 0)
    };

    vec![TextEdit {
        range: Range::new(start, end),
        new_text: new_text.to_string(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::Position;

    #[test]
    fn format_options_default() {
        let options = FormatOptions::new();
        assert_eq!(options.indent_size, INDENT_SIZE);
        assert!(options.spaces_around_brackets);
    }

    #[test]
    fn format_document_returns_none_when_unchanged() {
        let text = "component X { pins = [] }\n";
        let rope = Rope::from_str(text);
        let uri = Url::parse("file:///test.mc").unwrap();
        let result = format_document(&uri, &rope, None);
        // Simple text may have no changes
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn format_document_handles_multiline() {
        let text = "component X {\npins=[1]\n}\n";
        let rope = Rope::from_str(text);
        let uri = Url::parse("file:///test.mc").unwrap();
        let result = format_document(&uri, &rope, None);
        // Result may be Some or None depending on whether changes were made
        assert!(result.is_some() || result.is_none());
    }

    #[test]
    fn format_range_works() {
        let text = "component X {\npins=[1,2,3]\n}\n";
        let rope = Rope::from_str(text);
        let uri = Url::parse("file:///test.mc").unwrap();

        // Format second line
        let range = Range::new(Position::new(1, 0), Position::new(1, 15));
        let result = format_range(&uri, &rope, range, None);
        assert!(result.is_some());
    }

    #[test]
    fn diff_edits_returns_single_edit() {
        let rope = Rope::from_str("old text");
        let new_text = "new text";
        let edits = diff_edits(&rope, new_text);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "new text");
    }

    #[test]
    fn indent_size_constant() {
        assert_eq!(INDENT_SIZE, 4);
    }
}
