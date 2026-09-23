//! Folding ranges — brace-nested blocks folded to their opening line, computed
//! straight from the document text (the grammar's block faces: module/component/
//! interface/enum bodies, `if`/`match` arms, and any `{…}` block a library
//! feature adds). No lapper, no RPC: folding is a purely textual face.
//!
//! Comment runs (`//` line-comment blocks and `/* … */` spans) fold too, so the
//! bulk-collapse gesture works on header comments. String literals are skipped
//! so a brace or `//` inside a quoted value cannot open a phantom fold.

use ropey::Rope;
use tower_lsp::lsp_types::{FoldingRange, FoldingRangeKind};

/// A fold candidate: `[start_line, end_line]`, both 0-based and inclusive.
struct Span {
    start: u32,
    end: u32,
    kind: Fold,
}

enum Fold {
    Brace,
    Comment,
}

pub fn compute(rope: &Rope) -> Vec<FoldingRange> {
    let text = rope.slice(..).to_string();

    // Pass 1: brace nesting, with comments and strings opaque.
    let mut brace_spans = Vec::new();
    let mut brace_opens: Vec<u32> = Vec::new();
    {
        let bytes = text.as_bytes();
        let mut state = Top::Code;
        let mut line = 0u32;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    // A line comment ends at the newline; block comments and
                    // strings may span lines.
                    if state == Top::LineComment {
                        state = Top::Code;
                    }
                    line += 1;
                }
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    state = Top::LineComment;
                    i += 1;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    state = Top::BlockComment;
                    i += 1;
                }
                b'*' if bytes.get(i + 1) == Some(&b'/') && state == Top::BlockComment => {
                    state = Top::Code;
                    i += 1;
                }
                b'"' if state == Top::Code => state = Top::String,
                b'"' if state == Top::String => state = Top::Code,
                b'{' if state == Top::Code => brace_opens.push(line),
                b'}' if state == Top::Code => {
                    if let Some(open) = brace_opens.pop() {
                        if line > open {
                            brace_spans.push(Span {
                                start: open,
                                end: line,
                                kind: Fold::Brace,
                            });
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    // Pass 2: comment runs. A `//` run extends while each new line comment
    // starts on the line right after the previous one; `/* … */` folds as its
    // own span (multi-line only — a one-line block is not foldable).
    let mut comment_spans = Vec::new();
    {
        let mut run_start: Option<u32> = None;
        let mut run_end = 0u32;
        let mut state = Top::Code;
        let mut line = 0u32;
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    // A line comment run stays open across the newline only
                    // while the next line also starts a line comment.
                    if state == Top::LineComment {
                        state = Top::Code;
                        run_end = line;
                        // Look ahead: does the next line start with `//`?
                        let mut j = i + 1;
                        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                            j += 1;
                        }
                        if !(j < bytes.len()
                            && bytes[j] == b'/'
                            && bytes.get(j + 1) == Some(&b'/'))
                        {
                            if let Some(start) = run_start.take() {
                                if run_end > start {
                                    comment_spans.push(Span {
                                        start,
                                        end: run_end,
                                        kind: Fold::Comment,
                                    });
                                }
                            }
                        }
                    }
                    line += 1;
                }
                b'/' if bytes.get(i + 1) == Some(&b'/') && state == Top::Code => {
                    if run_start.is_none() {
                        run_start = Some(line);
                    }
                    state = Top::LineComment;
                    i += 1;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') && state == Top::Code => {
                    // Flush any line-comment run before the block comment.
                    if let Some(start) = run_start.take() {
                        if run_end > start {
                            comment_spans.push(Span {
                                start,
                                end: run_end,
                                kind: Fold::Comment,
                            });
                        }
                    }
                    state = Top::BlockComment;
                    let open_line = line;
                    i += 2;
                    while i < bytes.len() {
                        if bytes[i] == b'\n' {
                            line += 1;
                        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    if line > open_line {
                        comment_spans.push(Span {
                            start: open_line,
                            end: line,
                            kind: Fold::Comment,
                        });
                    }
                }
                b'"' => {
                    // Skip the string body in pass 2 as well (a `//` inside a
                    // quoted value must not open a run).
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        // Trailing run at EOF.
        if let Some(start) = run_start.take() {
            if run_end.max(line) > start {
                comment_spans.push(Span {
                    start,
                    end: run_end.max(line),
                    kind: Fold::Comment,
                });
            }
        }
    }

    let mut spans = brace_spans;
    spans.extend(comment_spans);
    spans.sort_by_key(|s| (s.start, s.end));
    spans.into_iter()
        .map(|s| FoldingRange {
            start_line: s.start,
            start_character: None,
            end_line: s.end,
            // Hide the closing line's content when collapsed.
            end_character: Some(0),
            kind: match s.kind {
                Fold::Brace => None, // "region" is the default
                Fold::Comment => Some(FoldingRangeKind::Comment),
            },
            collapsed_text: None,
        })
        .collect()
}

#[derive(PartialEq, Clone, Copy)]
enum Top {
    Code,
    LineComment,
    BlockComment,
    String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_braces_fold_per_level() {
        let r = Rope::from_str("module main {\n  if x {\n    y;\n  }\n}\n");
        let folds = compute(&r);
        assert_eq!(folds.len(), 2);
        // Sorted by opening line: outer block first.
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 4));
        assert_eq!((folds[1].start_line, folds[1].end_line), (1, 3));
        assert_eq!(folds[0].kind, None);
    }

    #[test]
    fn same_line_block_does_not_fold() {
        let r = Rope::from_str("f { a }; g { b\n};\n");
        let folds = compute(&r);
        assert_eq!(folds.len(), 1);
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 1));
    }

    #[test]
    fn line_comment_runs_and_block_comments_fold() {
        let r = Rope::from_str("// a\n// b\nlet x = 1;\n/* c\n   d */\n");
        let folds = compute(&r);
        assert_eq!(folds.len(), 2, "{folds:?}");
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 1));
        assert_eq!((folds[1].start_line, folds[1].end_line), (3, 4));
        assert!(folds
            .iter()
            .all(|f| f.kind == Some(FoldingRangeKind::Comment)));
    }

    #[test]
    fn single_line_comment_does_not_fold() {
        let r = Rope::from_str("// just one\nlet x = 1;\n");
        assert!(compute(&r).is_empty());
    }

    #[test]
    fn braces_inside_comments_and_strings_do_not_confuse_the_stack() {
        let r = Rope::from_str("// {\nmodule m {\n  let s = \"}\";\n}\n");
        let folds = compute(&r);
        assert_eq!(folds.len(), 1, "{folds:?}");
        assert_eq!((folds[0].start_line, folds[0].end_line), (1, 3));
    }

    #[test]
    fn unbalanced_brace_does_not_panic() {
        let r = Rope::from_str("module m {\n  let x = 1;\n");
        assert_eq!(compute(&r).len(), 0);
        let r = Rope::from_str("}\n");
        assert_eq!(compute(&r).len(), 0);
    }
}
