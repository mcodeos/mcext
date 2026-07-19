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
        if end <= rope.len_bytes() {
            let text = rope.byte_slice(pos..end).to_string();
            if is_mcode_keyword(&text) {
                type_map::T_KEYWORD
            } else {
                type_map::T_VARIABLE // identifier, not a keyword
            }
        } else {
            type_ as u32
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

// Tests disabled - require mcc direct calls
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::state::WorkspaceState;
//     use mcc::McURI;
//
//     fn fake_state(text: &str) -> (WorkspaceState, Url) {
//         let state = WorkspaceState::new();
//         let uri = Url::parse("file:///test.mc").unwrap();
//         state.insert_document(uri.clone(), Rope::from_str(text), 1);
//
//         let mc_uri = McURI::from("/test.mc");
//         mcc::mcc_load_from_string(&mc_uri, text);
//
//         if let Some(result) = mcc::mcc_query(&mc_uri) {
//             state.insert_parse(
//                 uri.clone(),
//                 std::sync::Arc::clone(&result.symbols.sem_tokens),
//                 std::sync::Arc::clone(&result.symbols.sem_symbols),
//                 mc_uri,
//             );
//         }
//
//         (state, uri)
//     }
