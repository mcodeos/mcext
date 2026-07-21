//! Find References + Rename — §15.2/§15.5
//!
//! LSP entry points: `textDocument/references`, `textDocument/rename`
//! Data source: RpcSemSymbols from sem RPC + RefDefMap reverse index

use crate::common::position::{offset_to_position, position_to_offset};
use crate::state::WorkspaceState;
use std::collections::HashMap;
use tower_lsp::lsp_types::{Location, Position, Range, TextEdit, Url};

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
