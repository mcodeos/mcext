//! Go to Definition — Jump to definition
//!
//! LSP entry point: `textDocument/definition`
//! Data source: RpcSemSymbols from sem RPC

use crate::common::position::{offset_to_position, position_to_offset};
use crate::rpc::{CrossFileTarget, LapperEntry, LocalDeclare, LocalReference};
use crate::state::{LocalDeclareSpan, RpcSemSymbols, WorkspaceState};
use ropey::Rope;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Range, Url};
use tracing::{info, trace, warn};

/// Compute goto definition response.
pub fn resolve(
    state: &WorkspaceState,
    uri: &Url,
    position: Position,
) -> Option<GotoDefinitionResponse> {
    info!("goto_def: enter uri={uri} pos={position:?}");

    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(position, &rope)?;

    let symbols_ref = state.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

    info!("goto_def: lapper_len={}, offset={}", symbols.lapper.len(), offset);

    // Find symbol at cursor position using lapper
    let intervals: Vec<_> = symbols.lapper.iter()
        .filter(|i| offset >= i.start && offset < i.stop)
        .collect();

    if intervals.is_empty() {
        info!("goto_def: no symbol found at offset {}", offset);
        return None;
    }

    // For simplicity, handle class_definition and declare_instance
    // More complex symbol types need additional mapping
    for interval in &intervals {
        match interval.kind.as_str() {
            "class_definition" => {
                // Find declaration for this class
                for decl in &symbols.local_declares {
                    if decl.id == interval.id {
                        return local_response(uri, decl.span, &rope);
                    }
                }
            }
            "declare_instance" => {
                for decl in &symbols.local_declares {
                    if decl.id == interval.id {
                        return local_response(uri, decl.span, &rope);
                    }
                }
            }
            "instance_reference" => {
                // Find declaration this references
                for decl in &symbols.local_declares {
                    if decl.id == interval.id {
                        return local_response(uri, decl.span, &rope);
                    }
                }
                // Try cross-file targets
                for target in &symbols.cross_file_targets {
                    if target.ref_id == interval.id {
                        return cross_file_response(state, &target.target_uri, target.span, &rope, uri);
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Same-file response: compute precise Range using local Rope
fn local_response(uri: &Url, span: [usize; 2], rope: &Rope) -> Option<GotoDefinitionResponse> {
    let start = offset_to_position(span[0], rope)?;
    let end = offset_to_position(span[1], rope)?;
    Some(GotoDefinitionResponse::Scalar(Location::new(
        uri.clone(),
        Range::new(start, end),
    )))
}

/// Cross-file response: read target file and compute Range
fn cross_file_response(
    state: &WorkspaceState,
    target_uri: &str,
    span: [usize; 2],
    current_rope: &Rope,
    current_uri: &Url,
) -> Option<GotoDefinitionResponse> {
    let target_url = Url::from_file_path(target_uri).ok()?;
    
    // Try to get rope from state or disk
    let target_rope = if let Some(r) = state.document_rope(&target_url) {
        r
    } else if target_url == *current_uri {
        current_rope.clone()
    } else {
        read_file_to_rope(&target_url)?
    };

    let start = offset_to_position(span[0], &target_rope)?;
    let end = offset_to_position(span[1], &target_rope)?;
    Some(GotoDefinitionResponse::Scalar(Location::new(
        target_url,
        Range::new(start, end),
    )))
}

fn read_file_to_rope(url: &Url) -> Option<Rope> {
    let path = url.to_file_path().ok()?;
    let content = std::fs::read_to_string(&path).ok()?;
    Some(Rope::from_str(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_for_missing_uri() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///missing.mc").unwrap();
        assert!(resolve(&state, &uri, Position::new(0, 0)).is_none());
    }
}
