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
    sorted_intervals.sort_by(|a, b| kind_rank(a.kind).cmp(&kind_rank(b.kind)));

    for interval in &sorted_intervals {
        info!(
            "goto_def: interval kind={}, id={}, start={}, stop={}, scope='{}'",
            interval.kind, interval.id, interval.start, interval.stop, interval.scope
        );
        // §4.2: All kinds → RefDefMap lookup (no fallback)
        let name = rope.byte_slice(interval.start..interval.stop).to_string();
        if let Some(resp) = resolve_ref_to_def(
            state,
            &symbols,
            &rope,
            uri,
            interval.kind,
            interval.id,
            &interval.scope,
            &name,
            (interval.start, interval.stop),
        ) {
            return Some(resp);
        }
        // ★ P6: Self-locate with correct file URI from lapper entry
        let def_uri_str = if !interval.file.is_empty() {
            &interval.file
        } else {
            uri.as_str()
        };
        return cross_file_response(
            state,
            def_uri_str,
            [interval.start, interval.stop],
            &rope,
            uri,
        );
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
                file: "file:///enum_test.mc".to_string(),
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
        let (state, uri) = state_with_lapper(source, vec![("EnumDef".into(), 0, 0, 9)]);
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
        let (state, uri) = state_with_lapper(source, vec![("EnumValDef".into(), 1, 11, 21)]);
        let response = resolve(&state, &uri, Position::new(1, 4));
        match response {
            Some(GotoDefinitionResponse::Array(v)) => assert!(v.is_empty()),
            other => panic!("expected empty Array for self-locate, got {other:?}"),
        }
    }

    #[test]
    fn enum_class_ref_miss_self_locate() {
        // §4.2: RefDefMap miss → self-locate (empty Array), never None.
        let source = "package = PKG.SOP8\n";
        let (state, uri) = state_with_lapper(source, vec![("EnumRef".into(), 7, 10, 13)]);
        let response = resolve(&state, &uri, Position::new(0, 11));
        match response {
            Some(GotoDefinitionResponse::Array(v)) if v.is_empty() => {}
            other => panic!("expected empty Array for RefDefMap miss, got {other:?}"),
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
        let (_state, _uri) = state_with_lapper(source, vec![("EnumValRef".into(), 99, 14, 18)]);

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

#[cfg(test)]
mod f12_e2e_tests {
    use super::*;
    use crate::rpc::{LapperEntry, RefDefEntryData, RefDefMapData};
    use crate::state::RpcSemSymbols;
    use std::sync::{Arc, Mutex};

    /// Standard kind_names matching SymbolKind ordinals from mcc.
    const KIND_NAMES: &[&str] = &[
        "ClassDef", "ClassRef", "InstDef", "InstRef", "PortDef", "PortRef",
        "LabelDef", "LabelRef", "FuncDef", "FuncRef",
        "PinIdDef", "PinIdRef", "PinNameDef", "PinNameRef", "PinIfaceDef", "PinIfaceRef",
        "EnumDef", "EnumRef", "EnumValDef", "EnumValRef",
        "RoleDef", "ParamDef", "DefineDef", "AttrDef",
    ];

    fn kind_ordinal(name: &str) -> u8 {
        KIND_NAMES.iter().position(|&k| k == name).unwrap() as u8
    }

    fn state_with_refdef(
        source: &str,
        lapper_entries: Vec<LapperEntry>,
        refdef_entries: Vec<(u8, u32, u32, u32, u32, u8)>, // (ref_kind, ref_id, file_id, def_span_start, def_span_end, def_kind)
    ) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str(source), 1);

        let ref_def_map = RefDefMapData {
            entries: refdef_entries
                .into_iter()
                .map(|(ref_kind, ref_id, file_id, ds, de, def_kind)| RefDefEntryData {
                    ref_kind,
                    ref_id,
                    file_id,
                    def_span: [ds, de],
                    def_kind,
                    container_id: 0,
                    cmie_kind: 255,
                })
                .collect(),
            files: vec!["file:///test.mc".to_string()],
            containers: vec!["".to_string()],
            func_names: vec![],
            kind_names: KIND_NAMES.iter().map(|s| s.to_string()).collect(),
            result_id: 0,
            index: std::sync::OnceLock::new(),
            kind_map: std::sync::OnceLock::new(),
        };

        let symbols = RpcSemSymbols {
            lapper: lapper_entries,
            local_declares: vec![],
            local_references: vec![],
            global_declares: vec![],
            global_references: vec![],
            ref_def_map: Some(ref_def_map),
        };
        state
            .symbols
            .sem_symbols
            .insert(uri.clone(), Arc::new(Mutex::new(symbols)));
        (state, uri)
    }

    /// Find line+col byte offset in source text.
    /// Find byte offset of a substring in source.
    fn byte_offset(source: &str, needle: &str, nth: usize) -> Option<usize> {
        source.match_indices(needle).nth(nth).map(|(i, _)| i)
    }

    fn pos_at(source: &str, offset: usize) -> Position {
        let rope = Rope::from_str(source);
        offset_to_position(offset, &rope).unwrap_or(Position::new(0, 0))
    }

    #[test]
    fn funcref_resolves_to_funcdef_via_refdefmap() {
        // Simulate: `func power(...)` def + `uC.power(...)` ref.
        // FuncRef(id=56) → FuncDef(id=56) via RefDefMap.
        let source = "func power(){}\n\nuC.power()\n";
        let def_start = byte_offset(source, "power", 0).unwrap(); // "power" in func power
        let def_end = def_start + 5;
        let ref_start = byte_offset(source, "power", 1).unwrap(); // "power" in uC.power
        let ref_end = ref_start + 5;

        let funcdef_kind = kind_ordinal("FuncDef");
        let funcref_kind = kind_ordinal("FuncRef");
        let lapper = vec![
            LapperEntry { kind: "FuncDef".into(), id: 56, start: def_start, stop: def_end, scope: "".into(), file: "file:///test.mc".into() },
            LapperEntry { kind: "FuncRef".into(), id: 56, start: ref_start, stop: ref_end, scope: "".into(), file: "file:///test.mc".into() },
        ];
        let refdef: Vec<(u8, u32, u32, u32, u32, u8)> = vec![
            (funcref_kind, 56, 0, def_start as u32, def_end as u32, funcdef_kind)
        ];
        let (state, uri) = state_with_refdef(source, lapper, refdef);

        // Cursor on "p" of "power" in uC.power()
        let resp = resolve(&state, &uri, pos_at(source, ref_start));
        match resp {
            Some(GotoDefinitionResponse::Scalar(loc)) => {
                assert_eq!(loc.uri, uri);
                let rope = Rope::from_str(source);
                let start = offset_to_position(def_start, &rope).unwrap();
                let end = offset_to_position(def_end, &rope).unwrap();
                assert_eq!(loc.range, Range::new(start, end));
            }
            other => panic!("expected Scalar, got {other:?}"),
        }
    }

    #[test]
    fn portref_resolves_to_portdef_via_refdefmap() {
        // Simulate: module M([VDD_3V3,GND]) with member VDD_3V3 used in funcall.
        let source = "module M([VDD_3V3,GND]){\n  uC.p([VDD_3V3,GND])\n}\n";
        let def_start = byte_offset(source, "VDD_3V3", 0).unwrap();
        let def_end = def_start + 7;
        let ref_start = byte_offset(source, "VDD_3V3", 1).unwrap();
        let ref_end = ref_start + 7;

        let portdef_kind = kind_ordinal("PortDef");
        let portref_kind = kind_ordinal("PortRef");
        let lapper = vec![
            LapperEntry { kind: "PortDef".into(), id: 100, start: def_start, stop: def_end, scope: "M".into(), file: "file:///test.mc".into() },
            LapperEntry { kind: "PortRef".into(), id: 100, start: ref_start, stop: ref_end, scope: "M".into(), file: "file:///test.mc".into() },
        ];
        let refdef: Vec<(u8, u32, u32, u32, u32, u8)> = vec![
            (portref_kind, 100, 0, def_start as u32, def_end as u32, portdef_kind)
        ];
        let (state, uri) = state_with_refdef(source, lapper, refdef);

        let resp = resolve(&state, &uri, pos_at(source, ref_start));
        match resp {
            Some(GotoDefinitionResponse::Scalar(loc)) => {
                assert_eq!(loc.uri, uri);
                let rope = Rope::from_str(source);
                let start = offset_to_position(def_start, &rope).unwrap();
                let end = offset_to_position(def_end, &rope).unwrap();
                assert_eq!(loc.range, Range::new(start, end));
            }
            other => panic!("expected Scalar for PortRef→PortDef, got {other:?}"),
        }
    }

    #[test]
    fn kind_rank_puts_funcref_before_instref() {
        // FuncRef=9, InstRef=3 — FuncRef should have higher priority (lower rank)
        assert!(kind_rank(9) <= kind_rank(3),
            "FuncRef must have priority >= InstRef to avoid shadowing");
    }

    #[test]
    fn all_known_kinds_have_explicit_rank() {
        // Verify all 24 SymbolKind ordinals have explicit rank entries.
        for kind in 0u8..24 {
            let rank = kind_rank(kind);
            assert!(rank < 7, "kind '{kind}' has default rank 7, needs explicit entry");
        }
    }
}

/// Unified ref→def resolution for handlers that follow the 4-level
/// scope-priority lookup. Returns `None` if unresolved (caller may
/// fall back to self-locate or error).
fn resolve_ref_to_def(
    state: &WorkspaceState,
    symbols: &crate::state::RpcSemSymbols,
    rope: &Rope,
    uri: &Url,
    kind: u8,
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
        if let Some(entry) = map.lookup(kind, id) {
            let def_uri = &map.files[entry.file_id as usize];
            eprintln!(
                "F12_DIAG resolve_ref_to_def name='{name}' kind={kind} id={id} \
                 => RefDefMap MATCH: uri={def_uri} span=[{},{}] def_kind={}",
                entry.def_span[0], entry.def_span[1], entry.def_kind
            );
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
        info!("resolve_ref_to_def: RefDefMap is None");
    }

    // No Level 1 fallback — RefDefMap is the single source of truth (§4.2)
    None
}

