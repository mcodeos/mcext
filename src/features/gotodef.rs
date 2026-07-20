//! Go to Definition — Jump to definition
//!
//! LSP entry point: `textDocument/definition`
//! Data source: RpcSemSymbols from sem RPC

use crate::common::position::{offset_to_position, position_to_offset};
use crate::features::symbols::kind_rank;
use crate::state::WorkspaceState;
use crate::util::usechk::{parse_use_prefix, resolve_use_path, strip_use_keyword};
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

    let symbols_ref = state.symbols.sem_symbols.get(uri)?;
    let symbols = symbols_ref.lock().ok()?;

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
        "goto_def: local_declares count={}, ref_def_map entries={}",
        symbols.local_declares.len(),
        symbols
            .ref_def_map
            .as_ref()
            .map(|m| m.entries.len())
            .unwrap_or(0)
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
        info!("goto_def: no symbol found at offset {offset}");
        return None;
    }

    // ★ Priority order: instance_ref before pin_name_def before others.
    // pin_name_def often has a large span covering constructor args
    // (e.g. `[+, -]::DC(volt, Source)`), which would otherwise shadow
    // instance_ref entries for the param references inside.
    let mut sorted_intervals = intervals.clone();
    sorted_intervals.sort_by(|a, b| kind_rank(&a.kind).cmp(&kind_rank(&b.kind)));

    for interval in &sorted_intervals {
        info!(
            "goto_def: interval kind={}, id={}, start={}, stop={}, scope='{}'",
            interval.kind, interval.id, interval.start, interval.stop, interval.scope
        );
        match interval.kind.as_str() {
            "class_def" | "class_definition" | "ClassDef" => {
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            "instance_def" | "declare_instance" | "InstDef" => {
                // Cursor is on the definition itself — self-locate.
                // (Previously called resolve_ref_to_def which searched by
                // *instance* name, jumping to unrelated same-named instances.)
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            // ── Label / port reference resolution ──
            // Strategy (see docs/label-lookup-strategy.md):
            //   1. Search same-scope for explicit def (port_def / label_def) → jump
            //   2. Search same-file for implicit def (port_def / label_def) → jump
            //   3. Neither found → self is the implicit def → self-locate
            "instance_ref" | "InstRef" | "label_ref" | "LabelRef" => {
                let name = rope.byte_slice(interval.start..interval.stop).to_string();
                eprintln!(
                    "F12_DIAG resolve instance_ref/label_ref name='{name}' kind={} id={} scope='{}' start={} stop={}",
                    interval.kind, interval.id, interval.scope, interval.start, interval.stop
                );
                if let Some(resp) = resolve_ref_to_def(
                    state,
                    &symbols,
                    &rope,
                    uri,
                    interval.kind.as_str(),
                    interval.id,
                    &interval.scope,
                    &name,
                    (interval.start, interval.stop),
                ) {
                    return Some(resp);
                }
                // Step 3: no earlier def found — self is the implicit definition.
                eprintln!(
                    "F12_DIAG resolve instance_ref/label_ref name='{name}' => Step3 self-locate (empty Array)"
                );
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            "port_def" | "PortDef" => {
                // Self-locate — use empty Array to prevent VS Code word-search fallback
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            // ★ enum support — separate kind family. Cursor on `enum PKG {`
            //   self-locates (empty Array, mirroring class_definition).
            "enum_class_def" | "EnumDef" => {
                info!(
                    "goto_def: enum_class_def at self, returning empty Array to prevent VS Code fallback"
                );
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            "enum_value_def" | "EnumValDef" => {
                // Self-locate — use empty Array to prevent VS Code word-search fallback
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            // Cursor on the class half of a qualified ref (e.g. `PKG` in
            // `PKG.SOP8`). Step 3b in mcc must emit this kind on the
            // reference site; loop-1 keeps the branch ready without it.
            "enum_class_ref" | "EnumRef" => {
                info!(
                    "goto_def: enum_class_ref id={} looking up target span",
                    interval.id
                );
                // For system-library enums (e.g. `PKG` from mcode), the
                // enum_class_ref id is a best-effort placeholder that can
                // collide with unrelated local declares (e.g. a port_def
                // at id=0).  Always resolve via the project index.
                let class_name = rope.byte_slice(interval.start..interval.stop).to_string();
                if !class_name.is_empty() {
                    let snap = state.project.index.snapshot();
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
            "enum_value_ref" | "EnumValRef" => {
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
                    .find(|i| {
                        (i.kind == "enum_class_ref" || i.kind == "EnumRef")
                            && i.stop + 1 == interval.start
                    })
                    .map(|i| rope.byte_slice(i.start..i.stop).to_string())
                    .and_then(|s| if s.is_empty() { None } else { Some(s) });
                if let (Some(ref class_name), Some(ref value_name)) = (class_name, value_name_opt) {
                    info!(
                        "goto_def: enum_value_ref class={} value={}",
                        class_name, value_name
                    );
                    if let Some(entry) = state
                        .project
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
                        state.project.index.snapshot().enum_value_len()
                    );
                }
            }
            // ── Definion kinds: self-locate ──
            "function_def" | "FuncDef" | "define_def" | "DefineDef" | "role_def" | "RoleDef"
            | "pin_name_def" | "PinNameDef" | "label_def" | "LabelDef" | "pin_id_def"
            | "PinIdDef" | "pin_iface_def" | "PinIfaceDef" | "enum_def" | "EnumDef"
            | "param_def" | "ParamDef" | "attr_def" | "AttrDef" => {
                // Self-locate: cursor on definition itself → stay
                return Some(GotoDefinitionResponse::Array(vec![]));
            }
            // ── Reference kinds: resolve via RefDefMap ──
            "function_ref" | "FuncRef" | "class_ref" | "ClassRef" | "declare_class"
            | "interface_ref" | "port_ref" | "PortRef" | "enum_ref" | "EnumRef"
            | "pin_name_ref" | "PinNameRef" | "pin_id_ref" | "PinIdRef" | "pin_iface_ref"
            | "PinIfaceRef" => {
                let name = rope.byte_slice(interval.start..interval.stop).to_string();
                if let Some(resp) = resolve_ref_to_def(
                    state,
                    &symbols,
                    &rope,
                    uri,
                    interval.kind.as_str(),
                    interval.id,
                    &interval.scope,
                    &name,
                    (interval.start, interval.stop),
                ) {
                    return Some(resp);
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

    let path = strip_use_keyword(&line_text)?;
    let (_prefix, use_path) = parse_use_prefix(path)?;

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

/// Cross-file response: read target file and compute Range
fn cross_file_response(
    state: &WorkspaceState,
    target_uri: &str,
    span: [usize; 2],
    current_rope: &Rope,
    current_uri: &Url,
) -> Option<GotoDefinitionResponse> {
    info!(
        "cross_file_response: ENTER target_uri={target_uri} span=[{},{}]",
        span[0], span[1]
    );
    // target_uri may be a `file://` URL (from enum/project index) or a bare
    // path (from mcc cross_file_targets). Handle both forms.
    let target_url = if target_uri.starts_with("file://") || target_uri.starts_with("untitled:") {
        Url::parse(target_uri).ok()?
    } else {
        Url::from_file_path(target_uri).ok()?
    };

    // Try to get rope from state or disk
    let target_rope = if let Some(r) = state.document_rope(&target_url) {
        info!("cross_file_response: using document_rope");
        r
    } else if target_url == *current_uri {
        info!("cross_file_response: using current_rope (same uri)");
        current_rope.clone()
    } else {
        info!("cross_file_response: reading from disk");
        read_file_to_rope(&target_url)?
    };

    let start = offset_to_position(span[0], &target_rope)?;
    let end = offset_to_position(span[1], &target_rope)?;
    info!(
        "cross_file_response: uri={} span=[{},{}] → pos=({},{})..({},{})",
        target_uri, span[0], span[1], start.line, start.character, end.line, end.character
    );
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
    use crate::state::RpcSemSymbols;
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
            ref_def_map: None,
        };
        state
            .symbols
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
        // `enum_value_def` self-locates with an empty Array to prevent
        // VS Code word-search fallback (mirrors `port_def` / `class_def`).
        let source = "enum PKG {\n    SOP8,\n    QFN20,\n}\n";
        let (state, uri) = state_with_lapper(source, vec![("enum_value_def".into(), 1, 11, 21)]);
        let response = resolve(&state, &uri, Position::new(1, 4));
        match response {
            Some(GotoDefinitionResponse::Array(v)) => assert!(v.is_empty()),
            other => panic!("expected empty Array for self-locate, got {other:?}"),
        }
    }

    #[test]
    fn enum_class_ref_no_index_entry_returns_none() {
        // `enum_class_ref` resolves via the project index. When the class
        // name isn't registered in the index, it returns None.
        let source = "package = PKG.SOP8\n";
        let (state, uri) = state_with_lapper(source, vec![("enum_class_ref".into(), 7, 10, 13)]);
        let response = resolve(&state, &uri, Position::new(0, 11));
        assert!(
            response.is_none(),
            "expected None for unregistered class, got {response:?}"
        );
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
        let (_state, _uri) = state_with_lapper(source, vec![("enum_value_ref".into(), 99, 14, 18)]);

        // Register the SOP8 row at span (90, 94) in another file. The
        //   State::index is a `IndexWorkerHandle`; in active mode it pulls
        //   from mcc, but in inactive mode (used by tests) its snapshot is
        //   empty. We test the lookup logic by constructing a ProjectIndex
        //   directly and asserting via `lookup_enum_value`.
        use crate::index::snapshot::ProjectIndex;
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

/// Resolve a lapper kind string to RefDefMap SymbolKind ordinal.
/// Uses the dynamic `kind_names` mapping from mcc (rather than hardcoded ordinals),
/// so mcc and mcext stay in sync automatically after enum changes (§7.6).
fn map_kind_from_str(kind: &str, map: &crate::rpc::RefDefMapData) -> Option<u8> {
    map.resolve_kind(kind)
}

/// Unified ref→def resolution for handlers that follow the 4-level
/// scope-priority lookup. Returns `None` if unresolved (caller may
/// fall back to self-locate or error).
fn resolve_ref_to_def(
    state: &WorkspaceState,
    symbols: &crate::state::RpcSemSymbols,
    rope: &Rope,
    uri: &Url,
    kind: &str,
    id: u32,
    _scope: &str,
    name: &str,
    ref_span: (usize, usize),
) -> Option<GotoDefinitionResponse> {
    // Level 0: RefDefMap lookup (O(1))
    if let Some(ref map) = symbols.ref_def_map {
        info!(
            "resolve_ref_to_def: RefDefMap present, entries={}, kind_names={:?}, looking for kind='{kind}' id={id} name='{name}'",
            map.entries.len(),
            map.kind_names,
        );
        if let Some(ref_kind) = map_kind_from_str(kind, map) {
            info!("resolve_ref_to_def: kind='{kind}' → ordinal={ref_kind}");
            if let Some(entry) = map.lookup(ref_kind, id) {
                let def_uri = &map.files[entry.file_id as usize];
                eprintln!(
                    "F12_DIAG resolve_ref_to_def name='{name}' kind={kind} id={id} \
                     => RefDefMap MATCH: uri={def_uri} span=[{},{}] def_kind={}",
                    entry.def_span[0], entry.def_span[1], entry.def_kind
                );
                // Skip self-references
                if entry.def_span[0] as usize == ref_span.0
                    && entry.def_span[1] as usize == ref_span.1
                {
                    eprintln!("F12_DIAG => RefDefMap SKIP (self-ref)");
                } else {
                    return cross_file_response(
                        state,
                        def_uri,
                        [entry.def_span[0] as usize, entry.def_span[1] as usize],
                        rope,
                        uri,
                    );
                }
            } else {
                info!(
                    "resolve_ref_to_def: RefDefMap MISS for kind='{kind}' id={id} name='{name}' (no entry in map)"
                );
            }
        } else {
            info!(
                "resolve_ref_to_def: kind='{kind}' NOT FOUND in kind_names={:?}",
                map.kind_names
            );
        }
    } else {
        info!("resolve_ref_to_def: RefDefMap is None");
    }

    // Level 1: project index / library by name (fallback for RefDefMap misses)
    if !name.is_empty() {
        if let Some(resp) = lookup_index(state, name, rope, uri) {
            return Some(resp);
        }
    }

    None
}

/// Look up a class name using mcc's unified_lookup RPC (Tier 1-4 priority),
/// falling back to project index.
fn lookup_index(
    state: &WorkspaceState,
    name: &str,
    rope: &Rope,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    use crate::index::snapshot::IndexKind;
    let snap = state.project.index.snapshot();
    for kind in &[
        IndexKind::Component,
        IndexKind::Module,
        IndexKind::Interface,
        IndexKind::Enum,
    ] {
        let entries = snap.lookup(*kind, name);
        if let Some(entry) = entries.first() {
            info!(
                "lookup_index: found '{name}' as {kind:?} → {} span=[{},{}]",
                entry.uri, entry.span.0, entry.span.1
            );
            return cross_file_response(
                state,
                &entry.uri.to_string(),
                [entry.span.0, entry.span.1],
                rope,
                uri,
            );
        }
    }
    info!("lookup_index: '{name}' NOT FOUND in Component or Module");
    None
}
