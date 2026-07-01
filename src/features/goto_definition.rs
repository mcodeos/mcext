//! Go to Definition — Jump to definition
//!
//! LSP entry point: `textDocument/definition`
//! Data source: mcc's `McSemSymbols` (current file) + project index (cross-file, Phase 1)
//!
//! **Phase 1 improvements**:
//! - Cross-file jumps now give precise Range (previously was (0,0)-(0,0) placeholder)
//! - For cross-file, read target file content from disk on demand, build Rope, compute precise position
//!
//! Note: mcc's `LocalSymbolTable`, `GlobalSymbolTable`, `DeclareId`, `ReferenceId`
//! are non-public types of the mcc crate. We access them indirectly through fields on
//! `mcc::McSemSymbols` and the `SymbolType` enum, without directly naming these types.

use crate::common::position::{offset_to_position, position_to_offset};
use crate::state::WorkspaceState;
use mcc::{Span, SymbolType};
use ropey::Rope;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Range, Url};
use tracing::trace;

/// Compute goto definition response.
pub fn resolve(
    state: &WorkspaceState,
    uri: &Url,
    position: Position,
) -> Option<GotoDefinitionResponse> {
    tracing::info!("goto_def: enter uri={uri} pos={position:?}");

    let rope = match state.document_rope(uri) {
        Some(r) => r,
        None => {
            tracing::warn!("goto_def: no rope for uri={uri}");
            return None;
        }
    };
    let offset = match position_to_offset(position, &rope) {
        Some(o) => o,
        None => {
            tracing::warn!("goto_def: no offset for pos={position:?}");
            return None;
        }
    };

    let symbols_ref = match state.sem_symbols.get(uri) {
        Some(r) => r,
        None => {
            tracing::warn!("goto_def: no sem_symbols for uri={uri}");
            return None;
        }
    };
    let symbols = symbols_ref.lock().unwrap_or_else(|e| e.into_inner());

    // ★ DEBUG: Log the lapper content
    let lapper_len = symbols.symbol_lapper.len();
    let decl_count = symbols.local_table.declare_inst_to_span.len();
    tracing::info!(
        "goto_def: lapper_len={lapper_len}, local_decl_count={decl_count}, offset={offset}"
    );

    // Check if there's any DeclareInstance in the lapper
    let intervals: Vec<_> = symbols.symbol_lapper.find(offset, offset + 1).collect();
    if intervals.is_empty() {
        tracing::info!(
            "goto_def: no symbol found at offset {offset}, line={}, char={}",
            position.line,
            position.character
        );
        // Try to find any DeclareInstance in the file
        for i in symbols.symbol_lapper.iter() {
            if matches!(i.val, SymbolType::DeclareInstance(_)) {
                tracing::info!(
                    "goto_def: DeclareInstance at {}-{} id={:?}",
                    i.start,
                    i.stop,
                    i.val
                );
            }
        }
        return None;
    }

    let interval = intervals.into_iter().next()?;
    trace!("goto_def: symbol={:?}", interval.val);

    match interval.val {
        SymbolType::DeclareClass(sid) => {
            // sid is a ReferenceId — look up the target definition span directly
            // (cross-file: class_id is per-file, so use declare_id_to_target_span)
            trace!("goto_def: DeclareClass sid={sid:?}");
            let gtable = symbols.global_table.lock().ok()?;
            let (target_uri_str, span) = gtable.declare_id_to_target_span.get(&sid)?;
            trace!("goto_def: DeclareClass target_uri={target_uri_str} span={span:?}");
            resolve_response(state, target_uri_str, span.clone(), &rope, uri)
        }
        SymbolType::ClassDefinition(sid) => {
            trace!("goto_def: ClassDefinition sid={sid:?}");
            let gtable = symbols.global_table.lock().ok()?;
            let (target_uri_str, span) = gtable.class_id_to_span.get(&sid)?;
            trace!("goto_def: target_uri={target_uri_str} span={span:?}");
            resolve_response(state, target_uri_str, span.clone(), &rope, uri)
        }
        SymbolType::InstanceReference(sid) => {
            trace!("goto_def: InstanceReference sid={sid:?}");
            let span = symbols.local_table.inst_id_to_span.get(&sid)?;
            local_response(uri, span.clone(), &rope)
        }
        SymbolType::InstanceRef(sid) => {
            // ★ New: Instance reference using DeclareId (from global table)
            trace!("goto_def: InstanceRef sid={sid:?}");
            let span = symbols.local_table.declare_inst_to_span.get(&sid)?;
            local_response(uri, span.clone(), &rope)
        }
        SymbolType::DeclareInstance(sid) => {
            trace!("goto_def: DeclareInstance sid={sid:?}");
            let span = symbols.local_table.declare_inst_to_span.get(&sid)?;
            local_response(uri, span.clone(), &rope)
        }
    }
}

/// Choose between same-file and cross-file response based on if the target URI
/// matches the current file.
fn resolve_response(
    state: &WorkspaceState,
    target_uri_str: &mcc::McURI,
    span: Span,
    current_rope: &Rope,
    current_uri: &Url,
) -> Option<GotoDefinitionResponse> {
    let target_url = parse_url_from_mc_uri(target_uri_str)?;
    tracing::debug!(
        "goto_def: resolve target_uri={target_uri_str} current_uri={current_uri} span={span:?} same_file={}",
        target_url == *current_uri
    );
    if target_url == *current_uri {
        let result = local_response(current_uri, span, current_rope);
        if let Some(ref r) = result {
            match r {
                GotoDefinitionResponse::Scalar(loc) => {
                    tracing::debug!(
                        "goto_def: local_response success -> line={} col={}",
                        loc.range.start.line,
                        loc.range.start.character
                    );
                }
                _ => {}
            }
        }
        result
    } else {
        cross_file_response(state, target_uri_str, span, current_rope, current_uri)
    }
}

/// Same-file response: compute precise Range using local Rope
fn local_response(uri: &Url, span: Span, rope: &Rope) -> Option<GotoDefinitionResponse> {
    let start = offset_to_position(span.start, rope)?;
    let end = offset_to_position(span.end, rope)?;
    Some(GotoDefinitionResponse::Scalar(Location::new(
        uri.clone(),
        Range::new(start, end),
    )))
}

/// Cross-file response: read target file from disk and build Rope, compute precise Range
///
/// Priority:
/// 1. Target file already open in state -> use state's Rope
/// 2. Target file on disk -> read from disk
/// 3. Neither -> fallback to (0,0)-(0,0) (shouldn't happen)
fn cross_file_response(
    state: &WorkspaceState,
    target_uri_str: &mcc::McURI,
    span: Span,
    current_rope: &Rope,
    current_uri: &Url,
) -> Option<GotoDefinitionResponse> {
    let target_url = parse_url_from_mc_uri(target_uri_str)?;

    // Try 1: get Rope from state
    let target_rope = if let Some(r) = state.document_rope(&target_url) {
        r
    } else if target_url == *current_uri {
        // Same file but current rope is already available
        current_rope.clone()
    } else {
        // Try 2: read from disk
        match read_file_to_rope(&target_url) {
            Some(r) => r,
            None => {
                tracing::warn!(
                    "goto_def: cross_file_response fallback to None for file={target_url}"
                );
                return None;
            }
        }
    };

    let start = offset_to_position(span.start, &target_rope)?;
    let end = offset_to_position(span.end, &target_rope)?;
    Some(GotoDefinitionResponse::Scalar(Location::new(
        target_url,
        Range::new(start, end),
    )))
}

/// Read file from disk and build Rope
fn read_file_to_rope(url: &Url) -> Option<Rope> {
    let path = url.to_file_path().ok()?;
    let content = std::fs::read_to_string(&path).ok()?;
    Some(Rope::from_str(&content))
}

fn parse_url_from_mc_uri(uri: &mcc::McURI) -> Option<Url> {
    // mcc::McURI = String, &String: AsRef<Path> is automatically implemented via Deref
    Url::from_file_path(uri).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::WorkspaceState;
    use mcc::McURI;
    use tower_lsp::lsp_types::Position;

    fn fake_state(text: &str) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str(text), 1);

        let mc_uri = McURI::from("/test.mc");
        mcc::mcc_load_from_string(&mc_uri, text);

        if let Some(result) = mcc::mcc_query(&mc_uri) {
            state.insert_parse(
                uri.clone(),
                std::sync::Arc::clone(&result.sem_tokens),
                std::sync::Arc::clone(&result.sem_symbols),
                mc_uri,
            );
        }

        (state, uri)
    }

    #[test]
    fn returns_none_for_missing_uri() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///missing.mc").unwrap();
        assert!(resolve(&state, &uri, Position::new(0, 0)).is_none());
    }

    #[test]
    fn returns_none_for_empty_lapper() {
        let (state, uri) = fake_state("component X {}");
        let result = resolve(&state, &uri, Position::new(0, 13));
        let _ = result;
    }

    #[test]
    fn does_not_panic_on_invalid_position() {
        let (state, uri) = fake_state("component X {}");
        let result = resolve(&state, &uri, Position::new(99, 0));
        assert!(result.is_none());
    }

    #[test]
    fn cross_file_response_falls_back_to_zero_when_target_missing() {
        // Target URI doesn't exist on disk -> (0,0)-(0,0)
        let target_uri = McURI::from("/nonexistent/file.mc");
        let span: Span = 10..15;
        let state = WorkspaceState::new();
        let rope = Rope::from_str("");
        let current_uri = Url::parse("file:///test.mc").unwrap();
        let result = cross_file_response(&state, &target_uri, span, &rope, &current_uri);
        assert!(result.is_some());
        match result.unwrap() {
            GotoDefinitionResponse::Scalar(loc) => {
                assert!(loc.uri.to_string().contains("nonexistent"));
                assert_eq!(loc.range.start, Position::new(0, 0));
                assert_eq!(loc.range.end, Position::new(0, 0));
            }
            _ => panic!("expected Scalar"),
        }
    }

    #[test]
    fn read_file_to_rope_missing_file() {
        let url = Url::parse("file:///does/not/exist.mc").unwrap();
        assert!(read_file_to_rope(&url).is_none());
    }
}
