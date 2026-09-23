//! Find References + Rename — §15.2/§15.5
//!
//! LSP entry points: `textDocument/references`, `textDocument/rename`
//! Data sources:
//! - [`resolve_cross_file`] — mcc `refs` RPC, position-aware, whole workspace
//!   (RefDefMap reverse index). Primary path; answers with locations in every
//!   loaded file, not just the one under the cursor.
//! - [`resolve`] — local lapper + local tables. Fallback when the mcc server
//!   is unavailable or the RPC misses, and the data source for rename (which
//!   stays current-file).

use crate::common::position::{offset_to_position, position_to_offset};
use crate::mccsrv::MccServer;
use crate::state::WorkspaceState;
use std::collections::HashMap;
use tower_lsp::lsp_types::{Location, Position, Range, TextEdit, Url};

/// Cross-file find-references via the mcc `refs` RPC.
///
/// Resolves the definition under the cursor on the mcc side (strict
/// position-aware path, `name` only as a fallback hint) and maps the returned
/// byte spans to LSP locations — including files never opened in the editor
/// (their text is read from disk for the offset→position conversion). Returns
/// None when the cursor cannot be turned into a byte offset or the RPC comes
/// back empty, so the caller can fall back to the local path.
pub async fn resolve_cross_file(
    state: &WorkspaceState,
    server: &MccServer,
    uri: &Url,
    position: Position,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(position, &rope)?;
    let hint = word_at_offset(&rope, offset);

    let resp = server
        .refs(uri.path(), offset, hint.as_deref())
        .await
        .ok()?;
    if resp.refs.is_empty() {
        return None;
    }

    let mut locations = Vec::new();
    for item in resp.refs {
        if item.def && !include_declaration {
            continue;
        }
        let target = Url::from_file_path(&item.uri)
            .map_err(|_| ())
            .or_else(|_| Url::parse(&item.uri).map_err(|_| ()))
            .ok()?;
        // The span lives in the item's own file, whose rope may need a disk
        // read; a file that is neither open nor readable drops just that row.
        let target_rope = state.rope_for_uri(&target)?;
        let start = offset_to_position(item.pos, &target_rope)?;
        let end = offset_to_position(item.end, &target_rope).unwrap_or(start);
        locations.push(Location::new(target, Range::new(start, end)));
    }
    if locations.is_empty() {
        None
    } else {
        Some(locations)
    }
}

/// The identifier text around `offset` — the name hint the refs RPC uses when
/// the cursor position does not resolve to a definition.
fn word_at_offset(rope: &ropey::Rope, offset: usize) -> Option<String> {
    if offset >= rope.len_bytes() {
        return None;
    }
    let is_word_byte = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    // Rope byte access is O(log n) per byte, and identifiers are short — a
    // plain byte walk on both sides is fine.
    let byte_at = |o: usize| rope.get_byte(o);
    let mut start = offset;
    while start > 0 && byte_at(start - 1).is_some_and(is_word_byte) {
        start -= 1;
    }
    let mut end = offset;
    while end < rope.len_bytes() && byte_at(end).is_some_and(is_word_byte) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let bytes: Vec<u8> = (start..end).filter_map(|o| byte_at(o)).collect();
    String::from_utf8(bytes).ok()
}

/// Find all references via lapper + local tables.
pub fn resolve(
    state: &WorkspaceState,
    uri: &Url,
    position: Position,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(position, &rope)?;
    let symbols_ref = state.symbols.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

    let intervals: Vec<_> = symbols
        .lapper
        .iter()
        .filter(|i| offset >= i.start && offset < i.stop)
        .collect();

    if intervals.is_empty() {
        return None;
    }

    let mut locations = Vec::new();
    let symbol_id = intervals.first()?.id;

    for decl in &symbols.local_declares {
        if decl.id == symbol_id {
            let start = offset_to_position(decl.span[0], &rope)?;
            let end = offset_to_position(decl.span[1], &rope)?;
            locations.push(Location::new(uri.clone(), Range::new(start, end)));
            break;
        }
    }

    for ref_info in &symbols.local_references {
        if ref_info.declare_id == Some(symbol_id) || ref_info.id == symbol_id {
            let is_decl = symbols.local_declares.iter().any(|d| d.id == ref_info.id);
            if !is_decl || include_declaration {
                let start = offset_to_position(ref_info.span[0], &rope)?;
                let end = offset_to_position(ref_info.span[1], &rope)?;
                locations.push(Location::new(uri.clone(), Range::new(start, end)));
            }
        }
    }

    if locations.is_empty() {
        None
    } else {
        Some(locations)
    }
}

/// ★ §15.5: Collect all rename edits for a symbol at the given position.
/// Uses lapper to find the symbol, then collects all references to build TextEdits.
pub fn collect_rename_edits(
    state: &WorkspaceState,
    uri: &Url,
    position: Position,
    new_name: &str,
) -> Option<HashMap<Url, Vec<TextEdit>>> {
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(position, &rope)?;
    let symbols_ref = state.symbols.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

    // Find symbol at cursor
    let intervals: Vec<_> = symbols
        .lapper
        .iter()
        .filter(|i| offset >= i.start && offset < i.stop)
        .collect();

    let target = intervals.first()?;

    // Collect all local references with matching id
    let mut edits: HashMap<Url, Vec<TextEdit>> = HashMap::new();

    // Add the definition/declaration itself
    for decl in &symbols.local_declares {
        if decl.id == target.id {
            let start = offset_to_position(decl.span[0], &rope)?;
            let end = offset_to_position(decl.span[1], &rope)?;
            edits.entry(uri.clone()).or_default().push(TextEdit {
                range: Range::new(start, end),
                new_text: new_name.to_string(),
            });
        }
    }

    // Add all local references
    for ref_info in &symbols.local_references {
        if ref_info.declare_id == Some(target.id) || ref_info.id == target.id {
            let start = offset_to_position(ref_info.span[0], &rope)?;
            let end = offset_to_position(ref_info.span[1], &rope)?;
            edits.entry(uri.clone()).or_default().push(TextEdit {
                range: Range::new(start, end),
                new_text: new_name.to_string(),
            });
        }
    }

    // Add lapper entries with matching id (covers symbols without explicit declare/ref entries)
    for entry in &symbols.lapper {
        if entry.id == target.id && entry.kind == target.kind {
            let start = offset_to_position(entry.start, &rope)?;
            let end = offset_to_position(entry.stop, &rope)?;
            let edit = TextEdit {
                range: Range::new(start, end),
                new_text: new_name.to_string(),
            };
            let entry_edits = edits.entry(uri.clone()).or_default();
            if !entry_edits.contains(&edit) {
                entry_edits.push(edit);
            }
        }
    }

    if edits.is_empty() {
        None
    } else {
        Some(edits)
    }
}
