//! Find References — Find references
//!
//! LSP entry point: `textDocument/references`
//! Data source: mcc's `McSemSymbols` (current file) + project index (cross-file, Phase 1)
//!
//! Phase 0 only in-file references; cross-file references added after Phase 1 project index.
//!
//! **Implementation note**: mcc's `SymbolType` different variants have different internal types
//! (`ReferenceId` vs `DeclareId`), and both types are not re-exported in mcc crate. We inline
//! all logic in match arms, relying on type inference to match HashMap key types.

use crate::common::position::position_to_offset;
use crate::state::WorkspaceState;
use mcc::{Span, SymbolType};
use tower_lsp::lsp_types::{Location, Position, Range, Url};

/// Compute references response.
///
/// `include_declaration` controls whether to include the declaration location itself.
pub fn resolve(
    state: &WorkspaceState,
    uri: &Url,
    position: Position,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(position, &rope)?;
    let symbols_ref = state.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().unwrap_or_else(|e| e.into_inner());

    let interval = symbols.symbol_lapper.find(offset, offset + 1).next()?;

    let mut entries: Vec<(mcc::McURI, Span, bool)> = Vec::new();

    match interval.val {
        SymbolType::DeclareClass(sid) => {
            // sid: ReferenceId (推断)
            let gtable = symbols.global_table.lock().ok()?;
            if include_declaration {
                if let Some((u, span)) = gtable.declare_class_id_to_span.get(&sid) {
                    entries.push((u.clone(), span.clone(), true));
                }
            }
            if let Some(class_id) = gtable.declare_id_to_class_id.get(&sid).copied() {
                if let Some(refs) = gtable.class_id_to_reference_ids.get(&class_id) {
                    for ref_id in refs {
                        if let Some((u, span)) = gtable.declare_class_id_to_span.get(ref_id) {
                            let is_decl = *ref_id == sid;
                            if !is_decl || include_declaration {
                                entries.push((u.clone(), span.clone(), is_decl));
                            }
                        }
                    }
                }
            }
        }
        SymbolType::ClassDefinition(sid) => {
            // sid: DeclareId (推断)
            let gtable = symbols.global_table.lock().ok()?;
            if include_declaration {
                if let Some((u, span)) = gtable.class_id_to_span.get(&sid) {
                    entries.push((u.clone(), span.clone(), true));
                }
            }
            if let Some(refs) = gtable.class_id_to_reference_ids.get(&sid) {
                for ref_id in refs {
                    if let Some((u, span)) = gtable.declare_class_id_to_span.get(ref_id) {
                        entries.push((u.clone(), span.clone(), false));
                    }
                }
            }
        }
        SymbolType::DeclareInstance(sid) => {
            // sid: DeclareId (推断)
            if let Some(span) = symbols.local_table.declare_inst_to_span.get(&sid) {
                entries.push((mcc::McURI::from(""), span.clone(), true));
            }
            if let Some(ref_ids) = symbols.local_table.declare_inst_to_inst_ids.get(&sid) {
                for ref_id in ref_ids {
                    if let Some(span) = symbols.local_table.inst_id_to_span.get(ref_id) {
                        entries.push((mcc::McURI::from(""), span.clone(), false));
                    }
                }
            }
        }
        SymbolType::InstanceReference(sid) => {
            // sid: ReferenceId (推断)
            if let Some(span) = symbols.local_table.inst_id_to_span.get(&sid) {
                entries.push((mcc::McURI::from(""), span.clone(), false));
            }
            if let Some(decl_id) = symbols
                .local_table
                .inst_id_to_declare_inst
                .get(&sid)
                .copied()
            {
                if let Some(decl_span) = symbols.local_table.declare_inst_to_span.get(&decl_id) {
                    entries.push((mcc::McURI::from(""), decl_span.clone(), true));
                }
            }
        }
    }

    if entries.is_empty() {
        return Some(Vec::new());
    }

    let mut locations = Vec::with_capacity(entries.len());
    for (loc_uri_str, span, is_decl) in entries {
        if !include_declaration && is_decl {
            continue;
        }
        let target_uri = parse_url_from_mc_uri(&loc_uri_str).unwrap_or_else(|| uri.clone());
        let range = span_to_range(state, &target_uri, span, uri, &rope)?;
        locations.push(Location::new(target_uri, range));
    }

    Some(locations)
}

/// 把 byte offset span 转 LSP Range
///
/// 优先级：
/// 1. 目标 = 当前文件 → 用本地 Rope
/// 2. 目标已在 state 中打开 → 用 state 的 Rope
/// 3. 目标在磁盘上 → 读磁盘构建 Rope
/// 4. 都没 → (0,0)-(0,0)
fn span_to_range(
    state: &WorkspaceState,
    target_uri: &Url,
    span: Span,
    current_uri: &Url,
    current_rope: &ropey::Rope,
) -> Option<Range> {
    use crate::common::position::offset_to_position;

    let target_rope = if target_uri == current_uri {
        current_rope.clone()
    } else if let Some(r) = state.document_rope(target_uri) {
        r
    } else if let Ok(path) = target_uri.to_file_path() {
        match std::fs::read_to_string(&path) {
            Ok(content) => ropey::Rope::from_str(&content),
            Err(_) => {
                return Some(Range::new(Position::new(0, 0), Position::new(0, 0)));
            }
        }
    } else {
        return Some(Range::new(Position::new(0, 0), Position::new(0, 0)));
    };

    let start = offset_to_position(span.start, &target_rope)?;
    let end = offset_to_position(span.end, &target_rope)?;
    Some(Range::new(start, end))
}

fn parse_url_from_mc_uri(uri: &mcc::McURI) -> Option<Url> {
    // mcc::McURI = String，&String: AsRef<Path> 通过 Deref 自动实现
    Url::from_file_path(uri).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::WorkspaceState;
    use mcc::McURI;
    use ropey::Rope;

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
    fn returns_none_for_missing_document() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///missing.mc").unwrap();
        let result = resolve(&state, &uri, Position::new(0, 0), true);
        assert!(result.is_none());
    }

    #[test]
    fn does_not_panic_on_real_mcode() {
        let (state, uri) = fake_state("component X { A - B }");
        for line in 0..3 {
            for col in 0..20 {
                let result = resolve(&state, &uri, Position::new(line, col), true);
                let _ = result;
            }
        }
    }

    #[test]
    fn returns_empty_or_some_for_no_symbol() {
        let (state, uri) = fake_state("xxx");
        let result = resolve(&state, &uri, Position::new(0, 1), false);
        assert!(result.is_none() || result.unwrap().is_empty());
    }
}
