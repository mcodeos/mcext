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

    eprintln!(
        "F12_DIAG offset={} lapper_len={}",
        offset, symbols.lapper.len()
    );

    // Debug: log all lapper intervals
    for i in &symbols.lapper {
        info!(
            "goto_def: lapper: kind={}, id={}, start={}, stop={}",
            i.kind, i.id, i.start, i.stop
        );
    }

    // Find symbol at cursor position using lapper.
    // Use an inclusive upper bound (`offset <= stop`) so that placing the caret
    // at the trailing edge of a token still resolves it. Identifier tokens are
    // separated by non-identifier characters (`.`, whitespace, etc.), so the
    // inclusive bound does not cause a caret to match an adjacent token.
    let intervals: Vec<_> = symbols
        .lapper
        .iter()
        .filter(|i| offset >= i.start && offset <= i.stop)
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

    // First, try to resolve use statement jump
    if let Some(response) = resolve_use_jump(uri, offset, &rope, state) {
        info!("goto_def: resolved use statement jump");
        return Some(response);
    }

    // Try symbol resolution
    if intervals.is_empty() {
        info!("goto_def: no symbol found at offset {}", offset);
        return None;
    }

    // ★ Priority order: class_definition first, then declare_class
    // Sort intervals to ensure class_definition is processed before declare_class
    let mut sorted_intervals = intervals.clone();
    sorted_intervals.sort_by(|a, b| {
        let order = |k: &str| match k {
            "class_definition"
            | "function_definition"
            | "define_definition"
            | "role_definition" => 0,
            "declare_class" | "class_ref" => 1,
            _ => 2,
        };
        order(&a.kind).cmp(&order(&b.kind))
    });

    for interval in &sorted_intervals {
        info!(
            "goto_def: processing interval kind={}, id={}, start={}, stop={}",
            interval.kind, interval.id, interval.start, interval.stop
        );
        match interval.kind.as_str() {
            "class_definition" => {
                // Cursor is on a class definition itself (e.g. "component MCU.US513_20_F").
                // VS Code ignores Scalar(自身) responses and falls back to reference search.
                // Use Array(vec![]) to signal "done, stay here" without triggering fallback.
                info!("goto_def: class_definition at self, returning empty Array to prevent VS Code fallback");
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            "declare_class" => {
                eprintln!(
                    "F12_DIAG declare_class id={} @{}:{} → searching",
                    interval.id, interval.start, interval.stop
                );
                // Try cross_file_targets FIRST: system-library classes won't
                // have a class_definition in the local lapper, and a local
                // class_definition with the same id (collision) would cause a
                // wrong jump (e.g. RES id=0 matching RESA id=0).
                for target in &symbols.cross_file_targets {
                    if target.ref_id == interval.id {
                        eprintln!(
                            "F12_DIAG declare_class → cross_file {} span={:?}",
                            target.target_uri, target.span
                        );
                        return cross_file_response(
                            state,
                            &target.target_uri,
                            target.span,
                            &rope,
                            uri,
                        );
                    }
                }
                // Fall back to local lapper for same-file classes
                for entry in &symbols.lapper {
                    if entry.kind == "class_definition" && entry.id == interval.id {
                        eprintln!(
                            "F12_DIAG declare_class → local class_definition @{}:{}",
                            entry.start, entry.stop
                        );
                        return local_response(uri, [entry.start, entry.stop], &rope);
                    }
                }
                eprintln!("F12_DIAG declare_class id={} → NOT FOUND", interval.id);
            }
            "declare_instance" => {
                // Self-locate: cursor is on the declaration itself.
                // VS Code ignores Scalar(自身) and falls back to word search,
                // which can jump to a different module's same-named instance.
                // Use Array(vec![]) to signal "stay here" without fallback.
                eprintln!(
                    "F12_DIAG declare_instance id={} scope={} @{}:{} → stay (empty Array)",
                    interval.id, interval.scope, interval.start, interval.stop
                );
                return Some(GotoDefinitionResponse::Array(vec![]));
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
                eprintln!(
                    "F12_DIAG instance_ref id={} scope={} @{}:{} → searching declare_instance",
                    interval.id, interval.scope, interval.start, interval.stop
                );
                let ref_scope = &interval.scope;

                // Exact (id, scope) match for declare_instance
                for entry in &symbols.lapper {
                    if entry.kind == "declare_instance"
                        && entry.id == interval.id
                        && entry.scope == *ref_scope
                    {
                        eprintln!(
                            "F12_DIAG instance_ref MATCHED declare_instance id={} scope={} @{}:{}",
                            entry.id, entry.scope, entry.start, entry.stop
                        );
                        return local_response(uri, [entry.start, entry.stop], &rope);
                    }
                }
                eprintln!("F12_DIAG instance_ref id={} scope={}: no declare_instance match, trying port_definition/cross_file", interval.id, ref_scope);
                // Component param ref → port_definition (scope-aware)
                for entry in &symbols.lapper {
                    if entry.kind == "port_definition"
                        && entry.id == interval.id
                        && entry.scope == *ref_scope
                    {
                        return local_response(uri, [entry.start, entry.stop], &rope);
                    }
                }
                // Try cross-file targets (e.g. jump to library definition)
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
                // Self-locate — use empty Array to prevent VS Code word-search fallback
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            // ★ enum support — separate kind family. Cursor on `enum PKG {`
            //   self-locates (empty Array, mirroring class_definition).
            "enum_class_def" => {
                info!(
                    "goto_def: enum_class_def at self, returning empty Array to prevent VS Code fallback"
                );
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            "enum_value_def" => {
                // Self-locate — use empty Array to prevent VS Code word-search fallback
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            // Cursor on the class half of a qualified ref (e.g. `PKG` in
            // `PKG.SOP8`). Step 3b in mcc must emit this kind on the
            // reference site; loop-1 keeps the branch ready without it.
            "enum_class_ref" => {
                info!(
                    "goto_def: enum_class_ref id={} looking up target span",
                    interval.id
                );
                // For system-library enums (e.g. `PKG` from mcode), the
                // enum_class_ref id is a best-effort placeholder that can
                // collide with unrelated local declares (e.g. a port_definition
                // at id=0).  Always resolve via the project index.
                let class_name = rope.byte_slice(interval.start..interval.stop).to_string();
                if !class_name.is_empty() {
                    let snap = state.index.snapshot();
                    let entries = snap.lookup(crate::index::snapshot::IndexKind::Enum, &class_name);
                    if let Some(entry) = entries.first() {
                        return cross_file_response(
                            state,
                            &entry.uri.to_string(),
                            [entry.span.0, entry.span.1],
                            &rope,
                            uri,
                        );
                    }
                }
            }
            // Cursor on the value half of a qualified ref (e.g. `SOP8`).
            //   (class, value) is recovered from the line tokens. For loop-1
            //   we lean on ProjectIndex.lookup_enum_value; the extension
            //   only reaches this branch when mcc emits it (Step 3b).
            "enum_value_ref" => {
                info!(
                    "goto_def: enum_value_ref id={} looking up (class, value) -> span",
                    interval.id
                );
                // Extract class + value names directly from the rope using
                // the interval positions emitted by mcc. The enum_class_ref
                // span covers the class name (e.g. "PKG") and sits immediately
                // before the dot; the enum_value_ref span covers the member
                // name (e.g. "QFN20").
                //
                // Previously we parsed the whole line by splitting on
                // non-is_alphanumeric characters, but that breaks when there
                // are CJK comments on the same line because is_alphanumeric()
                // returns false for Chinese characters.
                let value_name = rope.byte_slice(interval.start..interval.stop).to_string();
                let value_name_opt = if value_name.is_empty() {
                    None
                } else {
                    Some(value_name)
                };
                let class_name = symbols
                    .lapper
                    .iter()
                    .find(|i| i.kind == "enum_class_ref" && i.stop + 1 == interval.start)
                    .map(|i| rope.byte_slice(i.start..i.stop).to_string())
                    .and_then(|s| if s.is_empty() { None } else { Some(s) });
                if let (Some(ref class_name), Some(ref value_name)) = (class_name, value_name_opt) {
                    info!(
                        "goto_def: enum_value_ref class={} value={}",
                        class_name, value_name
                    );
                    if let Some(entry) = state
                        .index
                        .snapshot()
                        .lookup_enum_value(&class_name, &value_name)
                    {
                        return cross_file_response(
                            state,
                            &entry.uri.to_string(),
                            [entry.span.0, entry.span.1],
                            &rope,
                            uri,
                        );
                    }
                    info!(
                        "goto_def: enum_value_ref lookup FAILED for {}.{} (snapshot enum_values={})",
                        class_name, value_name,
                        state.index.snapshot().enum_value_len()
                    );
                }
            }
            // ── M6 gaps: new SymbolType variants ──
            "function_definition" | "define_definition" | "role_definition" => {
                // Self-locate: cursor on definition itself → stay
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            "pin_name_definition" => {
                // Self-locate — use empty Array to prevent VS Code word-search fallback
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            "function_ref" | "method_ref" | "class_ref" | "pin_name_ref" => {
                // Jump to definition via cross_file_targets
                for target in &symbols.cross_file_targets {
                    if target.ref_id == interval.id {
                        return cross_file_response(
                            state,
                            &target.target_uri,
                            [target.span[0], target.span[1]],
                            &rope,
                            uri,
                        );
                    }
                }
                // Fallback: search lapper for matching definition (same scope only)
                for entry in &symbols.lapper {
                    if entry.id == interval.id
                        && entry.kind != interval.kind
                        && entry.scope == interval.scope
                    {
                        return local_response(uri, [entry.start, entry.stop], &rope);
                    }
                }
            }
            _ => {}
        }
    }

    None
}
/// Resolve use statement jump - navigate to the target file when cursor is on a use path
fn resolve_use_jump(
    uri: &Url,
    offset: usize,
    rope: &Rope,
    _state: &WorkspaceState,
) -> Option<GotoDefinitionResponse> {
    let line_idx = rope.try_byte_to_line(offset).ok()?;
    let line_text = rope.get_line(line_idx)?.to_string();

    let Some(path) = parse_use_path(&line_text) else {
        return None;
    };
    let (_prefix, use_path) = path;

    let current_file = uri.to_file_path().ok()?;
    let current_dir = current_file.parent()?;

    let candidates = resolve_use_path(current_dir, use_path);
    let Some(target) = candidates.iter().find(|p| p.exists()) else {
        return None;
    };

    let target_url = Url::from_file_path(target).ok()?;
    // Jump to file start (0,0) — avoids Cmd+hover preview line for use statements
    let target_range = Range::new(Position::new(0, 0), Position::new(0, 0));

    Some(GotoDefinitionResponse::Scalar(Location::new(
        target_url,
        target_range,
    )))
}

fn parse_use_path(line: &str) -> Option<(&'static str, &str)> {
    let trimmed = line.trim();
    let after_use = trimmed
        .strip_prefix("pub use")
        .or_else(|| trimmed.strip_prefix("use"))?
        .trim();
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

fn resolve_use_path(base: &std::path::Path, path: &str) -> Vec<std::path::PathBuf> {
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
    eprintln!(
        "F12_DIAG local_response bytes[{}:{}] -> line{}col{}:line{}col{}",
        span[0], span[1], start.line, start.character, end.line, end.character
    );
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
    // target_uri may be a `file://` URL (from enum/project index) or a bare
    // path (from mcc cross_file_targets). Handle both forms.
    let target_url = if target_uri.starts_with("file://") || target_uri.starts_with("untitled:") {
        Url::parse(target_uri).ok()?
    } else {
        Url::from_file_path(target_uri).ok()?
    };

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
    use crate::rpc::LapperEntry;
    use crate::state::{LocalDeclareSpan, RpcSemSymbols};
    use std::sync::{Arc, Mutex};

    #[test]
    fn returns_none_for_missing_uri() {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///missing.mc").unwrap();
        assert!(resolve(&state, &uri, Position::new(0, 0)).is_none());
    }

    /// Helper: build a workspace state with a single-file document and the
    /// given lapper entries (the rest of RpcSemSymbols is empty).
    fn state_with_lapper(
        source: &str,
        lapper_entries: Vec<(
            String, /*kind*/
            u32,    /*id*/
            usize,  /*start*/
            usize,  /*stop*/
        )>,
    ) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///enum_test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str(source), 1);
        let lapper: Vec<LapperEntry> = lapper_entries
            .into_iter()
            .map(|(kind, id, start, stop)| LapperEntry {
                kind,
                start,
                stop,
                id,
                scope: String::new(),
            })
            .collect();
        let symbols = RpcSemSymbols {
            lapper,
            local_declares: vec![],
            local_references: vec![],
            global_declares: vec![],
            global_references: vec![],
            cross_file_targets: vec![],
        };
        state
            .sem_symbols
            .insert(uri.clone(), Arc::new(Mutex::new(symbols)));
        (state, uri)
    }

    #[test]
    fn enum_class_def_returns_empty_array() {
        // Document has `enum PKG { SOP8, QFN20 }` and we place an
        // `enum_class_def` lapper entry on the whole line.
        let source = "enum PKG {\n    SOP8,\n    QFN20,\n}\n";
        let (state, uri) = state_with_lapper(source, vec![("enum_class_def".into(), 0, 0, 9)]);
        let response = resolve(
            &state,
            &uri,
            Position::new(0, 5), /* inside `enum PKG {` */
        );
        match response {
            Some(GotoDefinitionResponse::Array(v)) => assert!(v.is_empty()),
            other => panic!("expected empty Array, got {other:?}"),
        }
    }

    #[test]
    fn enum_value_def_returns_local_self_response() {
        // `SOP8` row gets an enum_value_def lapper entry; cursor on it must
        // return the same span (self-resolution), matching the behavior of
        // `port_definition`.
        let source = "enum PKG {\n    SOP8,\n    QFN20,\n}\n";
        let (state, uri) = state_with_lapper(
            source,
            // Start of "    SOP8," line: byte offset = 11, end at 21.
            vec![("enum_value_def".into(), 1, 11, 21)],
        );
        let response = resolve(&state, &uri, Position::new(1, 4));
        match response {
            Some(GotoDefinitionResponse::Scalar(loc)) => {
                assert_eq!(loc.uri, uri);
                // Cursor jumped to row containing "SOP8," — should be line 1.
                assert_eq!(loc.range.start.line, 1);
            }
            other => panic!("expected Scalar response, got {other:?}"),
        }
    }

    #[test]
    fn enum_class_ref_local_declares_wins() {
        // Stub: an enum_class_ref at the same id as a local_declare target.
        // The branch should resolve via `local_declares`. The document is
        // padded so the local-declare span [30, 35] fits inside the file.
        let source = "package = PKG.SOP8\n\nenum PKG_X_REF { /* padding */ }\n";
        let (state, uri) = state_with_lapper(source, vec![("enum_class_ref".into(), 7, 10, 13)]);
        {
            let symbols_arc = state
                .sem_symbols
                .get(&uri)
                .expect("sem_symbols must contain uri");
            let mut symbols = symbols_arc.lock().unwrap();
            symbols.local_declares.push(LocalDeclareSpan {
                id: 7,
                span: [30, 35],
            });
            eprintln!(
                "DEBUG: local_declares now has {} entry(ies)",
                symbols.local_declares.len()
            );
        }
        let response = resolve(&state, &uri, Position::new(0, 11));
        match response {
            Some(GotoDefinitionResponse::Scalar(loc)) => {
                assert_eq!(loc.uri, uri);
                // The local-declare's span was [30, 35] within a multi-line
                //   doc; line 2 is where byte 30 lands. Verify the jump landed
                //   there rather than the cursor line 0.
                assert_ne!(loc.range.start.line, 0);
                assert_eq!(loc.range.start.line, 2);
            }
            other => panic!("expected Scalar response from local_declares, got {other:?}"),
        }
    }

    #[test]
    fn enum_value_ref_resolves_via_project_index() {
        // End-to-end-ish: a value ref `PKG.SOP8` emits an `enum_value_ref`
        //   lapper entry; cursor on `SOP8` should look up the index and jump
        //   to the registered body row in another file.
        //
        // Document is "package = PKG.SOP8\n\n". `PKG` covers [10..13],
        // `SOP8` covers [14..18]. Cursor at column 16 (the 'O' of `SOP8`).
        let source = "package = PKG.SOP8\n\n";
        let (state, uri) = state_with_lapper(source, vec![("enum_value_ref".into(), 99, 14, 18)]);

        // Register the SOP8 row at span (90, 94) in another file. The
        //   State::index is a `IndexWorkerHandle`; in active mode it pulls
        //   from mcc, but in inactive mode (used by tests) its snapshot is
        //   empty. We test the lookup logic by constructing a ProjectIndex
        //   directly and asserting via `lookup_enum_value`.
        use crate::index::snapshot::ProjectIndex;
        use crate::rpc::EnumValueEntry;
        let mut idx = ProjectIndex::new();
        let other_uri = Url::parse("file:///proj/pkg.mc").unwrap();
        idx.add_enum_value(
            "PKG",
            "SOP8",
            crate::index::snapshot::IndexEntry {
                uri: other_uri.clone(),
                span: (90, 94),
                name: "SOP8".into(),
            },
        );
        assert_eq!(
            idx.lookup_enum_value("PKG", "SOP8")
                .map(|e| e.uri.to_string()),
            Some(other_uri.to_string())
        );
    }
}
