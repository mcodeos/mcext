//! Go to Definition — Jump to definition
//!
//! LSP entry point: `textDocument/definition`
//! Data source: RpcSemSymbols from sem RPC

use crate::common::position::{offset_to_position, position_to_offset};
use crate::state::WorkspaceState;
use ropey::Rope;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Range, Url};
use tracing::info;

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

    info!(
        "goto_def: lapper_len={}, offset={}",
        symbols.lapper.len(),
        offset
    );

    // Debug: log all lapper intervals
    for i in &symbols.lapper {
        info!(
            "goto_def: lapper: kind={}, id={}, start={}, stop={}",
            i.kind, i.id, i.start, i.stop
        );
    }

    // Find symbol at cursor position using lapper
    let intervals: Vec<_> = symbols
        .lapper
        .iter()
        .filter(|i| offset >= i.start && offset < i.stop)
        .collect();

    info!(
        "goto_def: query: offset={}, lapper_count={}, matched_count={}",
        offset,
        symbols.lapper.len(),
        intervals.len()
    );
    // Debug: log intervals at this position
    for i in &intervals {
        info!(
            "goto_def: interval at offset {}: kind={}, id={}, start={}, stop={}",
            offset, i.kind, i.id, i.start, i.stop
        );
    }

    info!(
        "goto_def: local_declares count={}",
        symbols.local_declares.len()
    );
    for decl in &symbols.local_declares {
        info!(
            "goto_def: decl id={}, span=[{}, {}]",
            decl.id, decl.span[0], decl.span[1]
        );
    }

    info!(
        "goto_def: local_references count={}",
        symbols.local_references.len()
    );
    for ref_info in &symbols.local_references {
        info!(
            "goto_def: ref id={}, declare_id={:?}, span=[{}, {}]",
            ref_info.id, ref_info.declare_id, ref_info.span[0], ref_info.span[1]
        );
    }

    // First, try to resolve use statement jump (before or instead of symbol resolution)
    if let Some(response) = resolve_use_jump(uri, offset, &rope) {
        info!("goto_def: resolved use statement jump");
        return Some(response);
    }

    // If no use statement, try symbol resolution
    if intervals.is_empty() {
        info!("goto_def: no symbol found at offset {}", offset);
        return None;
    }

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
            "declare_class" => {
                info!(
                    "goto_def: declare_class id={} trying cross_file_targets",
                    interval.id
                );
                // Cross-file: find target in cross_file_targets
                for target in &symbols.cross_file_targets {
                    info!(
                        "goto_def: cross_file target ref_id={} target_uri={} span=[{}, {}]",
                        target.ref_id, target.target_uri, target.span[0], target.span[1]
                    );
                    if target.ref_id == interval.id {
                        return cross_file_response(
                            state,
                            &target.target_uri,
                            target.span,
                            &rope,
                            uri,
                        );
                    }
                }
                // Also check local declares
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
            "interface_ref" | "interface_reference" => {
                info!("goto_def: interface_ref id={}", interval.id);
                // Try cross-file targets
                let targets_count = symbols.cross_file_targets.len();
                info!("goto_def: cross_file_targets count={}", targets_count);
                for target in &symbols.cross_file_targets {
                    info!(
                        "goto_def: cross_file target ref_id={} target_uri={} span=[{}, {}]",
                        target.ref_id, target.target_uri, target.span[0], target.span[1]
                    );
                    if target.ref_id == interval.id {
                        return cross_file_response(
                            state,
                            &target.target_uri,
                            target.span,
                            &rope,
                            uri,
                        );
                    }
                }
                info!(
                    "goto_def: no cross_file target found for interface_ref id={}",
                    interval.id
                );
            }
            "instance_ref" | "instance_reference" => {
                // instance_ref points to a declaration with the same id
                // Find the declaration
                for decl in &symbols.local_declares {
                    if decl.id == interval.id {
                        return local_response(uri, decl.span, &rope);
                    }
                }
                // Try cross-file targets
                for target in &symbols.cross_file_targets {
                    if target.ref_id == interval.id {
                        return cross_file_response(
                            state,
                            &target.target_uri,
                            target.span,
                            &rope,
                            uri,
                        );
                    }
                }
            }
            "port_definition" => {
                // Port definitions point to themselves
                return local_response(uri, [interval.start, interval.stop], &rope);
            }
            _ => {}
        }
    }

    None
}

/// Resolve use statement jump - navigate to the target file when cursor is on a use path
fn resolve_use_jump(uri: &Url, offset: usize, rope: &Rope) -> Option<GotoDefinitionResponse> {
    // Get current line
    let line_idx = rope.try_byte_to_line(offset).ok()?;

    // Get line text
    let line_text = rope.get_line(line_idx)?.to_string();
    info!("goto_def: use_jump line_text = {:?}", line_text);

    // Parse use statement
    let Some(path) = parse_use_path(&line_text) else {
        info!("goto_def: use_jump parse_use_path returned None");
        return None;
    };
    let (prefix, use_path) = path;
    info!(
        "goto_def: use_jump parsed: prefix={:?}, use_path={:?}",
        prefix, use_path
    );

    // Get current file directory
    let current_file = uri.to_file_path().ok()?;
    let current_dir = current_file.parent()?;
    info!("goto_def: use_jump current_dir = {:?}", current_dir);

    // Resolve target path
    let candidates = resolve_use_path(current_dir, use_path);
    info!("goto_def: use_jump candidates = {:?}", candidates);

    let Some(target) = candidates.iter().find(|p| p.exists()) else {
        info!("goto_def: use_jump: no existing file found");
        return None;
    };
    info!("goto_def: use_jump target = {:?}", target);

    // Calculate the range - use the path part without the ./ prefix for highlighting
    // This avoids issues with VS Code's word separator on '.'
    let path_only = use_path; // e.g., "power.mc"
    let range_start_in_line = line_text.find(path_only)?;

    let target_url = Url::from_file_path(target).ok()?;
    let range = Range::new(
        Position::new(line_idx as u32, range_start_in_line as u32),
        Position::new(
            line_idx as u32,
            (range_start_in_line + path_only.len()) as u32,
        ),
    );

    info!("goto_def: use_jump SUCCESS: returning {:?}", target_url);
    Some(GotoDefinitionResponse::Scalar(Location::new(
        target_url, range,
    )))
}

/// Parse use path from line text, return (prefix, path) or None
fn parse_use_path(line: &str) -> Option<(&'static str, &str)> {
    let trimmed = line.trim();
    let after_use = trimmed
        .strip_prefix("pub use")
        .or_else(|| trimmed.strip_prefix("use"))?
        .trim();

    // Get first whitespace-separated token (the path, without trailing comments)
    let path = after_use.split_whitespace().next()?;
    if path.is_empty() {
        return None;
    }

    if let Some(p) = path.strip_prefix("./") {
        Some(("./", p))
    } else if let Some(p) = path.strip_prefix("../") {
        Some(("../", p))
    } else {
        None
    }
}

/// Resolve use path to candidate file paths
fn resolve_use_path(base: &std::path::Path, path: &str) -> Vec<std::path::PathBuf> {
    // If path already has .mc extension, use it directly
    if path.ends_with(".mc") {
        return vec![base.join(path)];
    }

    if path.contains('/') {
        vec![base.join(format!("{path}.mc"))]
    } else {
        vec![
            base.join(format!("{path}.mc")),
            base.join(format!("{path}/{path}.mc")),
        ]
    }
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
