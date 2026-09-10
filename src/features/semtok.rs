//! Semantic Tokens — Semantic highlighting
//!
//! LSP entry points:
//! - `textDocument/semanticTokens/full` (implemented)
//! - `textDocument/semanticTokens/range` (implemented)
//! - `textDocument/semanticTokens/delta` (Phase 3 implementation)
//!
//! Data source: mcc's `McSemTokens`, each token contains `(type_, position, length)`.
//! Conversion rules see `doc/features/highlight.md`.
//!
//! This module exposes:
//! - [`compute`] calculates full tokens
//! - [`compute_delta`] calculates incremental diff

use crate::common::legend::type_map;
use crate::state::WorkspaceState;
use ropey::Rope;
use tower_lsp::lsp_types::{SemanticToken, Url};

/// mcc-returned type value for multi-line comment tokens.
/// Value 101 is outside the LSP legend range (0..16) and mapped to
/// `type_map::T_COMMENT` after splitting into per-line tokens.
const MULTILINE_COMMENT_TYPE: i16 = 101;

/// Compute semantic tokens for the document corresponding to URI (delta-encoded).
///
/// Returns `Vec<SemanticToken>`, sorted by position in ascending order.
pub fn compute(state: &WorkspaceState, uri: &Url) -> Option<Vec<SemanticToken>> {
    let rope = state.document_rope(uri)?;
    let tokens_ref = state.symbols.sem_tokens.get(uri)?;
    let tokens_guard = tokens_ref.lock().unwrap_or_else(|e| {
        tracing::warn!("sem_tokens lock poisoned, attempting recovery");
        e.into_inner()
    });

    // Copy + sort (mcc doesn't guarantee order)
    let mut sorted = tokens_guard.tokens.clone();
    sorted.sort_by_key(|t| t.position);

    let mut out = Vec::with_capacity(sorted.len());
    let mut last_line: u32 = 0;
    let mut last_start: u32 = 0;

    for token in sorted {
        if token.position < 0 || token.length <= 0 {
            continue;
        }

        // Multi-line comments need to be split by line
        if token.type_ == MULTILINE_COMMENT_TYPE {
            emit_multiline_comment(
                &rope,
                token.position,
                token.length,
                &mut last_line,
                &mut last_start,
                &mut out,
            );
            continue;
        }

        // Regular token
        emit_single_token(
            &rope,
            token.type_,
            token.position,
            token.length,
            &mut last_line,
            &mut last_start,
            &mut out,
        );
    }

    Some(out)
}

fn emit_single_token(
    rope: &Rope,
    type_: i16,
    position: i32,
    length: i32,
    last_line: &mut u32,
    last_start: &mut u32,
    out: &mut Vec<SemanticToken>,
) {
    // Skip invalid tokens
    if position < 0 || length <= 0 {
        return;
    }

    let pos = position as usize;
    let len = length as usize;
    let rope_len = rope.len_bytes();

    // Skip tokens that are clearly out of bounds
    if pos >= rope_len || pos.saturating_add(len) > rope_len {
        return;
    }

    let end = pos.saturating_add(len);

    let line = match rope.try_byte_to_line(pos) {
        Ok(l) => l as u32,
        Err(_) => return,
    };
    let line_start_char = match rope.try_line_to_char(line as usize) {
        Ok(c) => c as u32,
        Err(_) => return,
    };
    let current_char_pos = match rope.try_byte_to_char(pos) {
        Ok(c) => c as u32,
        Err(_) => return,
    };
    let start = current_char_pos - line_start_char;

    let delta_line = line - *last_line;
    let delta_start = if delta_line == 0 {
        start - *last_start
    } else {
        start
    };

    *last_line = line;
    *last_start = start;

    // Reclassify KEYWORD-typed identifiers: check if it's a real language keyword
    let final_type = if type_ == type_map::T_KEYWORD as i16 {
        match extract_aligned_text(rope, pos, end) {
            Some(text) if is_mcode_keyword(&text) => type_map::T_KEYWORD,
            Some(_) => type_map::T_VARIABLE, // identifier, not a keyword
            // Range doesn't map to whole chars (stale/mis-measured token data
            // against the current buffer) — keep mcc's classification.
            None => type_ as u32,
        }
    } else {
        type_ as u32
    };

    out.push(SemanticToken {
        delta_line,
        delta_start,
        length: length as u32,
        token_type: final_type,
        token_modifiers_bitset: 0,
    });
}

/// Returns the document text covered by byte range `[start, end)`, or `None`
/// when the range cannot be mapped to whole UTF-8 chars.
///
/// mcc reports token positions/lengths as byte offsets, but those can drift out
/// of alignment with the live `Rope` (the analysis snapshot races the buffer).
/// `ropey`'s `byte_slice` panics on ranges that split a multi-byte char, while
/// its `try_byte_to_char`/`try_byte_to_line` silently round interior bytes down
/// to the containing char — so neither is a usable boundary check.  Instead we
/// first try the non-panicking `get_byte_slice`, then, when a boundary cuts a
/// char, snap inward to the maximal char-aligned sub-range still inside the
/// token (dropping the partial edge chars) so the keyword reclassification
/// below can still run on a sane prefix.
fn extract_aligned_text(rope: &Rope, start: usize, end: usize) -> Option<String> {
    // Fast path: range already falls on char boundaries (in-sync, ASCII, ...).
    if let Some(s) = rope.get_byte_slice(start..end) {
        return Some(s.to_string());
    }

    let rope_len = rope.len_bytes();
    if start >= rope_len {
        return None;
    }
    let end = end.min(rope_len);

    // A byte index is on a char boundary iff it is at the rope end or its byte
    // is not a UTF-8 continuation byte (0x80..0xBF).
    let is_boundary = |b: usize| b >= rope_len || (rope.byte(b) & 0xC0) != 0x80;

    // Move `start` up to the next boundary (excludes the char cut at the left).
    let mut lo = start;
    while !is_boundary(lo) {
        lo += 1;
    }
    // Move `end` down to the last boundary strictly inside the range.
    let mut hi = end;
    while hi > lo && !is_boundary(hi) {
        hi -= 1;
    }

    if hi <= lo {
        return None;
    }
    // `lo`/`hi` are char boundaries now, so this cannot panic.
    Some(rope.byte_slice(lo..hi).to_string())
}

/// Known mcode language keywords
fn is_mcode_keyword(text: &str) -> bool {
    matches!(
        text,
        "module"
            | "component"
            | "interface"
            | "enum"
            | "func"
            | "if"
            | "else"
            | "use"
            | "pub"
            | "as"
            | "in"
            | "io"
            | "ps"
            | "nc"
            | "anl"
            | "out"
            | "this"
            | "role"
            | "pins"
            | "int"
            | "float"
            | "string"
            | "bool"
            | "true"
            | "false"
            | "return"
    )
}

/// Compute incremental diff between two token lists.
///
/// Returns [`SemanticTokensDelta`]:
/// - `edits`: edit sequence based on prev, transitioning to curr
/// - If diff cost exceeds full, returns `None` (caller should use full)
///
/// Algorithm: simple line alignment (O(n)), suitable for most edit scenarios.
/// For large-scale rewrites, delta may be larger; in that case fallback to full.
pub fn compute_delta(
    prev: &[SemanticToken],
    curr: &[SemanticToken],
) -> Option<tower_lsp::lsp_types::SemanticTokensDelta> {
    // Special handling for empty list
    if curr.is_empty() {
        if prev.is_empty() {
            return Some(tower_lsp::lsp_types::SemanticTokensDelta {
                edits: vec![],
                result_id: None,
            });
        }
        // Delete all: generate delete edit
        return Some(tower_lsp::lsp_types::SemanticTokensDelta {
            edits: vec![tower_lsp::lsp_types::SemanticTokensEdit {
                start: 0,
                delete_count: prev.len() as u32,
                data: None,
            }],
            result_id: None,
        });
    }

    // Estimate delta cost: edits + remaining tokens vs full
    // If too many edits, return full directly
    // Note: delete_count doesn't count in transmission cost (just delete command), only newly inserted data needs transmission
    let edits = compute_edits(prev, curr);
    let edit_cost: usize = edits
        .iter()
        .map(|e| e.data.as_ref().map_or(0, |d| d.len()))
        .sum();
    let full_cost = curr.len();
    if edit_cost > full_cost {
        return None;
    }

    Some(tower_lsp::lsp_types::SemanticTokensDelta {
        edits,
        result_id: None,
    })
}

/// Internal: compute edit sequence from prev to curr
fn compute_edits(
    prev: &[SemanticToken],
    curr: &[SemanticToken],
) -> Vec<tower_lsp::lsp_types::SemanticTokensEdit> {
    let mut edits = Vec::new();
    let mut prev_idx: usize = 0;
    let mut curr_idx: usize = 0;

    while curr_idx < curr.len() {
        if prev_idx >= prev.len() {
            // All remaining are new
            edits.push(tower_lsp::lsp_types::SemanticTokensEdit {
                start: prev_idx as u32,
                delete_count: 0,
                data: Some(curr[curr_idx..].to_vec()),
            });
            break;
        }

        if prev[prev_idx] == curr[curr_idx] {
            // Match, skip
            prev_idx += 1;
            curr_idx += 1;
            continue;
        }

        // No match: find longest suffix match
        let (p, c) = find_best_match(prev, prev_idx, curr, curr_idx);
        if p > prev_idx || c > curr_idx {
            // Has common prefix that can be kept
            let del_count = (p - prev_idx) as u32;
            let ins_data = curr[curr_idx..c].to_vec();
            edits.push(tower_lsp::lsp_types::SemanticTokensEdit {
                start: prev_idx as u32,
                delete_count: del_count,
                data: if ins_data.is_empty() {
                    None
                } else {
                    Some(ins_data)
                },
            });
            prev_idx = p;
            curr_idx = c;
        } else {
            // 没有匹配，替换单个 token
            edits.push(tower_lsp::lsp_types::SemanticTokensEdit {
                start: prev_idx as u32,
                delete_count: 1,
                data: Some(vec![curr[curr_idx]]), // SemanticToken is Copy
            });
            prev_idx += 1;
            curr_idx += 1;
        }
    }

    // 处理剩余的 prev tokens（应该被删除）
    if prev_idx < prev.len() {
        edits.push(tower_lsp::lsp_types::SemanticTokensEdit {
            start: prev_idx as u32,
            delete_count: (prev.len() - prev_idx) as u32,
            data: None,
        });
    }

    edits
}

/// 找最长公共后缀（贪心匹配下一个相同 token）
fn find_best_match(
    prev: &[SemanticToken],
    prev_start: usize,
    curr: &[SemanticToken],
    curr_start: usize,
) -> (usize, usize) {
    let mut best_p = prev_start;
    let mut best_c = curr_start;

    // 从后往前找最长匹配段
    let mut p = prev.len();
    let mut c = curr.len();

    while p > prev_start && c > curr_start {
        p -= 1;
        c -= 1;
        if prev[p] == curr[c] {
            best_p = p + 1;
            best_c = c + 1;
        } else {
            break;
        }
    }

    (best_p, best_c)
}

fn emit_multiline_comment(
    rope: &Rope,
    start_byte: i32,
    length: i32,
    last_line: &mut u32,
    last_start: &mut u32,
    out: &mut Vec<SemanticToken>,
) {
    // Skip invalid tokens
    if start_byte < 0 || length <= 0 {
        return;
    }

    let start = start_byte as usize;
    let len = length as usize;
    let rope_len = rope.len_bytes();

    // Skip tokens that are clearly out of bounds
    if start >= rope_len || start.saturating_add(len) > rope_len {
        return;
    }

    // Clamp end to rope boundaries
    let end = (start + len).min(rope_len);
    if end <= start {
        return;
    }

    let start_line = match rope.try_byte_to_line(start) {
        Ok(l) => l,
        Err(_) => return,
    };
    // Clamp end_line to valid range
    let end_line = match rope.try_byte_to_line(end.saturating_sub(1)) {
        Ok(l) => l,
        Err(_) => return,
    };

    for line_idx in start_line..=end_line {
        // Defensive: skip if line_idx is out of bounds
        if line_idx >= rope.len_lines() {
            break;
        }

        let line_u32 = line_idx as u32;
        let line_start_char = match rope.try_line_to_char(line_idx) {
            Ok(c) => c as u32,
            Err(_) => break,
        };
        let line_byte_offset = match rope.try_line_to_byte(line_idx) {
            Ok(o) => o,
            Err(_) => break,
        };
        let char_in_line = start.max(line_byte_offset);
        let current_char_pos = match rope.try_byte_to_char(char_in_line) {
            Ok(c) => c as u32,
            Err(_) => break,
        };
        let start_col = current_char_pos - line_start_char;
        let line_str = rope.line(line_idx).to_string();
        let line_len = line_str.len().saturating_sub(start_col as usize) as u32;

        let delta_line = line_u32 - *last_line;
        let delta_start = if delta_line == 0 {
            start_col - *last_start
        } else {
            start_col
        };

        *last_line = line_u32;
        *last_start = start_col;

        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: line_len,
            token_type: type_map::T_COMMENT,
            token_modifiers_bitset: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RpcSemTokens, SemTokenEntry};
    use ropey::Rope;
    use std::sync::{Arc, Mutex};

    fn state_with_tokens(text: &str, raw_tokens: Vec<(i16, i32, i32)>) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str(text), 1);
        let tokens = RpcSemTokens {
            tokens: raw_tokens
                .into_iter()
                .map(|(type_, position, length)| SemTokenEntry {
                    type_,
                    position,
                    length,
                })
                .collect(),
        };
        state
            .symbols
            .sem_tokens
            .insert(uri.clone(), Arc::new(Mutex::new(tokens)));
        (state, uri)
    }

    // ── compute() tests ──

    #[test]
    fn empty_tokens_returns_some_empty() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str("x\n"), 1);
        let tokens = RpcSemTokens { tokens: vec![] };
        state
            .symbols
            .sem_tokens
            .insert(uri.clone(), Arc::new(Mutex::new(tokens)));
        let result = compute(&state, &uri).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn single_token_delta_encoded() {
        let (state, uri) = state_with_tokens("abc\n", vec![(0, 0, 3)]);
        let result = compute(&state, &uri).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].delta_line, 0);
    }

    #[test]
    fn tokens_sorted_by_position() {
        let (state, uri) = state_with_tokens("abcdef\n", vec![(1, 4, 2), (0, 0, 3)]);
        let result = compute(&state, &uri).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].token_type, 0);
    }

    #[test]
    fn keyword_kept_for_real_keyword() {
        let (state, uri) = state_with_tokens("component X\n", vec![(13, 0, 9)]);
        let result = compute(&state, &uri).unwrap();
        assert!(result.iter().any(|t| t.token_type == type_map::T_KEYWORD));
    }

    #[test]
    fn keyword_reclassified_to_variable() {
        let (state, uri) = state_with_tokens("foobar\n", vec![(13, 0, 6)]);
        let result = compute(&state, &uri).unwrap();
        assert!(
            result.iter().all(|t| t.token_type != type_map::T_KEYWORD),
            "non-keyword should not have KEYWORD type"
        );
    }

    // Regression: a KEYWORD-typed token whose byte range cuts through a
    // multi-byte UTF-8 char (stale mcc token data vs. the live buffer) used to
    // panic inside ropey's byte_slice. It must degrade to a token instead.
    #[test]
    fn multibyte_split_span_does_not_panic() {
        // "ab中文cd\n": CJK chars occupy bytes 2..8, so a byte range ending
        // mid-char no longer has both ends on char boundaries.
        let text = "ab中文cd\n";
        let (state, uri) = state_with_tokens(text, vec![(13, 1, 6)]);
        let result = compute(&state, &uri).unwrap();
        assert_eq!(result.len(), 1);
        // The recoverable prefix "b中" is not a keyword.
        assert_eq!(result[0].token_type, type_map::T_VARIABLE);
    }

    #[test]
    fn multibyte_split_start_does_not_panic() {
        // "a中bc\n": token starts on byte 2, which is a continuation byte of
        // the 3-byte CJK char 中.
        let text = "a中bc\n";
        let (state, uri) = state_with_tokens(text, vec![(13, 2, 3)]);
        let result = compute(&state, &uri).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].token_type, type_map::T_VARIABLE);
    }

    #[test]
    fn aligned_keyword_after_multibyte_kept() {
        // A real ASCII keyword on a line that also holds CJK earlier must still
        // be classified as KEYWORD (both byte ends are on char boundaries).
        let text = "//中注释\ncomponent X\n";
        let pos = text.find("component").unwrap() as i32;
        let (state, uri) = state_with_tokens(text, vec![(13, pos, 9)]);
        let result = compute(&state, &uri).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].token_type, type_map::T_KEYWORD);
    }

    #[test]
    fn negative_position_skipped() {
        let (state, uri) = state_with_tokens("abc\n", vec![(0, -1, 3)]);
        let result = compute(&state, &uri).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn out_of_bounds_token_skipped() {
        let (state, uri) = state_with_tokens("abc\n", vec![(0, 100, 3)]);
        let result = compute(&state, &uri).unwrap();
        assert!(result.is_empty());
    }

    // ── Delta tests ──

    #[test]
    fn delta_empty_prev_and_curr() {
        let prev: Vec<SemanticToken> = vec![];
        let curr: Vec<SemanticToken> = vec![];
        let delta = compute_delta(&prev, &curr).unwrap();
        assert!(delta.edits.is_empty());
    }

    #[test]
    fn delta_delete_all() {
        let prev = vec![make_token(0, 0, 3, 0)];
        let curr = vec![];
        let delta = compute_delta(&prev, &curr).unwrap();
        assert_eq!(delta.edits.len(), 1);
        assert_eq!(delta.edits[0].delete_count, 1);
    }

    #[test]
    fn delta_insert_all() {
        let prev = vec![];
        let curr = vec![make_token(0, 0, 3, 0)];
        let delta = compute_delta(&prev, &curr).unwrap();
        assert_eq!(delta.edits.len(), 1);
        assert!(delta.edits[0].data.is_some());
    }

    fn make_token(line: u32, start: u32, len: u32, tt: u32) -> SemanticToken {
        SemanticToken {
            delta_line: line,
            delta_start: start,
            length: len,
            token_type: tt,
            token_modifiers_bitset: 0,
        }
    }
}
