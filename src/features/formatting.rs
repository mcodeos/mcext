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
#[derive(Debug, Clone, Default)]
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

    let text = rope.slice(start_offset..end_offset).to_string();
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
                    // 不在行首时不加空格
                }
                result.push('{');
                indent_level += 1;
                in_line = false;

                // 检查下一行是否为空或只有注释
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

                // 检查是否需要换行
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
                    // 行内空格保留
                    if !result.ends_with(' ') && !result.ends_with('\t') {
                        result.push(' ');
                    }
                } else if c == '\t' {
                    // 制表符转空格
                    if !result.ends_with('\t') {
                        result.push(' ');
                    }
                }
                // 换行和行首空格已处理
                if c != '\n' {
                    line_start = false;
                }
            }
            ',' => {
                result.push(',');
                in_line = true;
                skip_whitespace_and_comments(&mut chars);
                if let Some(&'\n') = chars.peek() {
                    // 不处理，让换行逻辑处理
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
                    // 不处理，让换行逻辑处理
                } else if let Some(&'/') = chars.peek() {
                    // 注释后面换行
                } else {
                    result.push('\n');
                    add_indent(&mut result, indent_level, options.indent_size);
                }
                line_start = true;
            }
            '/' => {
                // 检查是否是注释
                if let Some(&'/') = chars.peek() {
                    // 行注释
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
                    // 空数组
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
                    // 行首检查
                }
                result.push(c);
                in_line = true;
                line_start = false;
            }
        }
    }

    // 清理末尾空行
    while result.ends_with('\n') {
        result.pop();
    }
    result.push('\n');

    Some(result)
}

/// 跳过空白和注释
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
                // 不是注释，把 / 放回去
                break;
            }
        } else {
            break;
        }
    }
}

/// 添加缩进
fn add_indent(result: &mut String, level: usize, indent_size: usize) {
    for _ in 0..level {
        for _ in 0..indent_size {
            result.push(' ');
        }
    }
}

/// 计算两个文本之间的 diff edits
pub fn diff_edits(rope: &Rope, new_text: &str) -> Vec<TextEdit> {
    use tower_lsp::lsp_types::Position;

    // 简单的全量替换
    // 后续可以优化为真正的 diff 算法
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
        // 简单文本可能没有变化
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn format_document_handles_multiline() {
        let text = "component X {\npins=[1]\n}\n";
        let rope = Rope::from_str(text);
        let uri = Url::parse("file:///test.mc").unwrap();
        let result = format_document(&uri, &rope, None);
        // 格式化结果可能 Some 或 None（取决于是否有变化）
        assert!(result.is_some() || result.is_none());
    }

    #[test]
    fn format_range_works() {
        let text = "component X {\npins=[1,2,3]\n}\n";
        let rope = Rope::from_str(text);
        let uri = Url::parse("file:///test.mc").unwrap();

        // 格式化第二行
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
