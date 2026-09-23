//! Hover — Show symbol information on hover
//!
//! LSP entry point: `textDocument/hover`
//!
//! Two modes:
//!   (1) Use-statement hover — shows component/interface/module/enum definitions
//!       in the target file, using the project index.
//!   (2) Symbol hover — shows definition info for any symbol tracked by
//!       the semantic lapper (class, port, label, enum value, func, etc.).
//!
//! Shares data sources with gotodef: sem-symbols lapper + project index.

use crate::index::snapshot::{IndexEntry, IndexKind};
use crate::state::WorkspaceState;
use crate::util::usechk::{parse_use_prefix, resolve_use_target, strip_use_keyword};
use ropey::Rope;
use tower_lsp::lsp_types::{Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, Url};

/// Maximum number of definition entries to display in a hover tooltip.
const MAX_ENTRIES: usize = 8;

/// The `show.*` subject a hover resolved to — the same drill-down
/// completionItem/resolve grounds with (`comp::ground_item`).
pub struct Subject {
    /// `show.<kind>` kind word: component / module / interface / enum.
    pub kind: &'static str,
    pub name: String,
}

// ============================================================================
// Public entry point
// ============================================================================

/// Resolve hover information for a position, plus the `show.*` subject it
/// resolved to (when the symbol is one of the four named kinds).
pub fn resolve_with_subject(
    state: &WorkspaceState,
    params: &HoverParams,
) -> (Option<Hover>, Option<Subject>) {
    let uri = &params.text_document_position_params.text_document.uri;
    let position = params.text_document_position_params.position;
    let Some(rope) = state.document_rope(uri) else {
        return (None, None);
    };
    let Some(offset) = crate::common::position::position_to_offset(position, &rope) else {
        return (None, None);
    };
    let subject = hover_subject(state, uri, offset);

    // ── (1) Use-statement hover ──
    if let Some(hover) = resolve_use_hover(&rope, offset, uri, state) {
        return (Some(hover), None);
    }

    // ── (2) Symbol hover ──
    if let Some(hover) = resolve_symbol_hover(state, uri, &rope, offset) {
        return (Some(hover), subject);
    }

    (None, None)
}

/// Resolve hover information for a position.
pub fn resolve(state: &WorkspaceState, params: &HoverParams) -> Option<Hover> {
    resolve_with_subject(state, params).0
}

/// Determine which `show.*` drill-down the hovered symbol maps to. Defs are
/// disambiguated through the project index (a ClassDef is any of the four
/// CMIE kinds); refs carry the exact kind in the RefDefMap's `cmie_kind`.
/// Anything else — ports, pins, funcs, locals — has no `show.*` target.
fn hover_subject(state: &WorkspaceState, uri: &Url, offset: usize) -> Option<Subject> {
    let (info, name) = crate::features::symbols::find_symbol_at_offset(state, uri, offset)?;
    match info.kind {
        // ClassDef — one SymbolKind covering component/module/interface/enum.
        0 => {
            let snap = state.project.index.snapshot();
            for (ik, kind) in [
                (IndexKind::Component, "component"),
                (IndexKind::Module, "module"),
                (IndexKind::Interface, "interface"),
                (IndexKind::Enum, "enum"),
            ] {
                if snap.lookup(ik, &name).iter().any(|e| e.uri == *uri) {
                    return Some(Subject {
                        kind,
                        name: name.to_string(),
                    });
                }
            }
            None
        }
        // EnumDef
        16 => Some(Subject {
            kind: "enum",
            name: name.to_string(),
        }),
        // ClassRef / EnumRef / EnumValRef — the def map carries the CMIE kind
        // and mcc's own def-name capture.
        1 | 17 | 19 => {
            let map = {
                let cell = state.symbols.sem_symbols.get(uri)?;
                let guard = cell.lock().ok()?;
                guard.ref_def_map.clone()?
            };
            let entry = map.lookup(info.kind, info.id)?;
            let kind = cmie_label(entry.cmie_kind)?;
            let def_name = if entry.def_name.is_empty() {
                name.to_string()
            } else {
                entry.def_name.clone()
            };
            Some(Subject { kind, name: def_name })
        }
        _ => None,
    }
}

/// completionItem/resolve grounds with `comp::markdown`; hover does the same:
/// append the `show.<kind>` card below the local hover (definition line +
/// location stay first — they are the cheapest, most precise facts). Any RPC
/// failure keeps the local hover untouched: grounding is decoration, never a
/// reason to lose what we already resolved.
pub async fn ground(hover: Hover, subject: &Subject, rpc: &crate::rpc::MccRpcClient) -> Hover {
    let resp = match rpc.show(subject.kind, &subject.name).await {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(
                "hover: show.{}({}) failed: {e}",
                subject.kind,
                subject.name
            );
            return hover;
        }
    };
    let card = crate::features::comp::markdown(subject.kind, &subject.name, &resp);
    let HoverContents::Markup(mut markup) = hover.contents else {
        return hover;
    };
    if markup.value.trim().is_empty() {
        markup.value = card;
    } else {
        markup.value.push_str("\n\n---\n\n");
        markup.value.push_str(&card);
    }
    Hover {
        contents: HoverContents::Markup(markup),
        range: hover.range,
    }
}

// ============================================================================
// (1) Use-statement hover
// ============================================================================

/// Build hover for `use ./path` statements — list all public definitions
/// in the target file using the project index snapshot.
fn resolve_use_hover(
    rope: &Rope,
    offset: usize,
    uri: &Url,
    state: &WorkspaceState,
) -> Option<Hover> {
    let line_idx = rope.try_byte_to_line(offset).ok()?;
    let line_text = rope.get_line(line_idx)?.to_string();
    let trimmed = line_text.trim();

    // Only trigger on use / pub use lines
    let path_str = strip_use_keyword(trimmed)?;
    let (_prefix, use_path) = parse_use_prefix(path_str)?;

    // Only handle relative paths for now
    if !path_str.starts_with("./") && !path_str.starts_with("../") {
        return None;
    }

    // Resolve target URL
    let target_url = resolve_use_target(uri, use_path)?;

    // Query index for all definitions in the target file
    let entries = lookup_file_entries(state, &target_url);

    // Build hover content
    let file_label = target_url
        .to_file_path()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| target_url.to_string());

    let content = if entries.is_empty() {
        format_markdown(&format!("📁 `{}`", file_label), &[])
    } else {
        let def_lines: Vec<String> = entries
            .iter()
            .take(MAX_ENTRIES)
            .map(|e| format_entry_line(e))
            .collect();

        let header = format!("📁 `{}`  — {} definition(s)", file_label, entries.len());
        format_markdown(&header, &def_lines)
    };

    Some(Hover {
        contents: HoverContents::Markup(content),
        range: None,
    })
}

/// Collect all index entries whose URI matches the target file.
fn lookup_file_entries(state: &WorkspaceState, target_url: &Url) -> Vec<IndexEntry> {
    let snap = state.project.index.snapshot();
    snap.lookup_file(target_url)
        .into_iter()
        .map(|(_kind, entry)| entry.clone())
        .collect()
}

// ============================================================================
// (2) Symbol hover
// ============================================================================

/// Build hover for a semantic symbol at the cursor position.
///
/// Uses the shared `find_symbol_at_offset` (same data source as gotodef) to
/// find which symbol is under the cursor, then looks up its definition in the
/// project index or cross-file-targets table.
fn resolve_symbol_hover(
    state: &WorkspaceState,
    uri: &Url,
    _rope: &Rope,
    offset: usize,
) -> Option<Hover> {
    let (info, name) = crate::features::symbols::find_symbol_at_offset(state, uri, offset)?;

    match info.kind {
        // Self-defining symbols — show their type
        0 | 8 | 4 | 22 | 20 | 16 | 18 | 10 | 12 | 14 | 6 => {
            format_symbol_hover(&name, &info.kind_label, &info.scope)
        }
        // Reference symbols — try to resolve to definition
        1 | 3 | 17 | 19 | 9 | 11 | 13 | 15 | 7 => {
            // Carry the RefDefMap (same source as F12) so pin/port/label refs
            // can show their definition instead of a bare resolved name.
            let ref_def_map = {
                let cell = state.symbols.sem_symbols.get(uri)?;
                let guard = cell.lock().ok()?;
                guard.ref_def_map.clone()
            };
            resolve_reference_hover(
                state,
                uri,
                &name,
                info.kind,
                info.id,
                &info.scope,
                ref_def_map.as_ref(),
            )
        }
        // Instance definitions / declarations
        2 => format_symbol_hover(&name, "instance", &info.scope),
        _ => None,
    }
}

/// Resolve a reference (ref kind) to its definition for hover display.
fn resolve_reference_hover(
    state: &WorkspaceState,
    current_uri: &Url,
    name: &str,
    kind: u8,
    id: u32,
    scope: &str,
    ref_def_map: Option<&crate::rpc::RefDefMapData>,
) -> Option<Hover> {
    let snap = state.project.index.snapshot();

    // Determine the index kind to search
    let index_kind = match kind {
        1 => Some(IndexKind::Component), // ClassRef
        17 => Some(IndexKind::Enum),     // EnumRef
        _ => None,
    };

    // ★ RefDefMap lookup — precise def file + span (same source as F12).
    if let Some(map) = ref_def_map {
        if let Some(entry) = map.lookup(kind, id) {
            if let Some(hover) = resolve_defmap_hover(state, current_uri, name, kind, entry, map) {
                return Some(hover);
            }
        }
    }

    // Try index lookup first
    if let Some(ik) = index_kind {
        let entries = snap.lookup(ik, name);
        if let Some(entry) = entries.first() {
            let source = entry
                .uri
                .to_file_path()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_default();
            let ref_kind = kind_label(kind);
            let mut lines = vec![
                format!("`{}` ({})", name, ref_kind),
                format!("📄 {}", source),
            ];
            // ★ Full definition first — same source-line lead as the RefDefMap
            // path. Best-effort: an unreadable def file (e.g. a dependency
            // outside the workspace) just keeps the bare arrow lines.
            if let Some(rope) = state
                .document_rope(&entry.uri)
                .or_else(|| read_file_to_rope(&entry.uri))
            {
                if let Some(line) = def_text_from_rope(&rope, entry.span.0) {
                    lines.insert(0, format!("```\n{}\n```", line));
                }
            }
            return format_markdown_hover(&lines);
        }
    }

    // Fallback: show scope + name
    if !scope.is_empty() {
        let lines = vec![
            format!("`{}` — reference", name),
            format!("scope: `{}`", scope),
        ];
        format_markdown_hover(&lines)
    } else {
        format_symbol_hover(name, kind_label(kind), "")
    }
}

/// Build a hover from a RefDefMap def entry: definition file + the source
/// line containing the def (e.g. `io [16, 17, 21] = ADC::ADC.DIFF(Receiver)`).
fn resolve_defmap_hover(
    state: &WorkspaceState,
    current_uri: &Url,
    name: &str,
    kind: u8,
    entry: &crate::rpc::RefDefEntryData,
    map: &crate::rpc::RefDefMapData,
) -> Option<Hover> {
    let def_uri_str = map.files.get(entry.file_id as usize)?;
    let def_url = if def_uri_str.starts_with("file://") || def_uri_str.starts_with("untitled:") {
        Url::parse(def_uri_str).ok()?
    } else {
        Url::from_file_path(def_uri_str).ok()?
    };

    let def_rope = if let Some(r) = state.document_rope(&def_url) {
        r
    } else if def_url == *current_uri {
        state.document_rope(current_uri)?
    } else {
        read_file_to_rope(&def_url)?
    };

    let start = entry.def_span[0] as usize;
    let pos = crate::common::position::offset_to_position(start, &def_rope)?;
    // ★ Full definition — the source line containing the def span, e.g.
    // `io 7 = NRST, "NRST"` for a pin, `component RES(v, r)` for a
    // component, or the func signature line. `entry.def_name` carries only
    // the bare symbol name (no params / pin numbers); the enclosing line is
    // the user's own complete declaration, which is what a hover leads with.
    let def_text = def_text_from_rope(&def_rope, start)?;

    let file_label = def_url
        .to_file_path()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| def_uri_str.clone());
    // ★ CMIE kind (0=Component, 1=Module, 2=Interface, 3=Enum) is more precise
    // than the ref SymbolKind for class refs: a `::DC` ref whose def is an
    // `interface` must tag as `interface`, not a generic `class`. Fall back to
    // the SymbolKind label when the kind is unknown (255).
    let ref_kind = cmie_label(entry.cmie_kind).unwrap_or_else(|| kind_label(kind));
    // Definition first, then the resolved symbol and its location.
    let lines = vec![
        format!("```\n{}\n```", def_text),
        format!("`{}` ({})", name, ref_kind),
        format!("📄 {}:{}", file_label, pos.line + 1),
    ];
    format_markdown_hover(&lines)
}

/// Read a file into a rope (cross-file def lookup).
fn read_file_to_rope(url: &Url) -> Option<Rope> {
    let path = url.to_file_path().ok()?;
    let content = std::fs::read_to_string(&path).ok()?;
    Some(Rope::from_str(&content))
}

/// Extract the user's complete definition text at `byte_offset` — the source
/// line containing the def, extended across continuation lines until the
/// `()`/`[]` brackets balance: `component CAP(\n    cap::UV.CAP, ...)` for a
/// multi-line component header, `io 7 = NRST, "NRST"` for a pin, or a func
/// signature line. The bare symbol name alone (no params / pin numbers) isn't
/// the definition, so the enclosing declaration text leads the hover.
fn def_text_from_rope(rope: &Rope, byte_offset: usize) -> Option<String> {
    const MAX_LINES: usize = 15;
    const MAX_CHARS: usize = 300;

    let start_line = rope.try_byte_to_line(byte_offset).ok()?;
    let mut depth: i32 = 0;
    let mut out: Vec<String> = Vec::new();
    let end_line = (start_line + MAX_LINES).min(rope.len_lines());

    for line_idx in start_line..end_line {
        let line = rope.get_line(line_idx)?.to_string();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        for c in line.chars() {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                _ => {}
            }
        }
        out.push(trimmed.to_string());
        if depth <= 0 {
            break;
        }
    }

    let mut text = out.join("\n");
    if text.chars().count() > MAX_CHARS {
        text = text.chars().take(MAX_CHARS).collect::<String>();
        text.push('…');
    }
    Some(text)
}

// ============================================================================
// Helpers
// ============================================================================

/// Human-readable label for a SymbolKind ordinal — no `→` prefix; the hover
/// already leads with the full definition line, so the kind tag is plain
/// (`pin`, `instance`, …) rather than a reference arrow.
fn kind_label(kind: u8) -> &'static str {
    // SymbolKind ordinals from mcc
    match kind {
        0 => "component/module", // ClassDef
        1 => "class",            // ClassRef
        2 => "instance",         // InstDef
        3 => "instance",         // InstRef
        4 => "port",             // PortDef
        5 => "port",             // PortRef
        6 => "label",            // LabelDef
        7 => "label",            // LabelRef
        8 => "function",         // FuncDef
        9 => "function",         // FuncRef
        10 | 12 | 14 => "pin",   // Pin*Def
        11 | 13 | 15 => "pin",   // Pin*Ref
        16 | 18 => "enum",       // EnumDef/EnumValDef
        17 | 19 => "enum",       // EnumRef/EnumValRef
        20 => "role",            // RoleDef
        21 => "param",           // ParamDef
        22 => "define",          // DefineDef
        23 => "attr",            // AttrDef
        24 => "func param",      // FuncParamRef
        25 => "bus",             // BusDef
        26 => "bus",             // BusRef
        27 => "unknown",         // UnknownDef
        28 => "bus member",      // BusMemberDef
        29 => "bus member",      // BusMemberRef
        _ => "?",
    }
}

/// Label from a CMIE kind ordinal (RefDefEntryData.cmie_kind) — the real kind
/// of the class/def (0=Component, 1=Module, 2=Interface, 3=Enum), plain
/// (no `→` prefix, matching `kind_label`). Returns None for UNKNOWN (255) so
/// callers fall back to the SymbolKind label.
fn cmie_label(cmie_kind: u8) -> Option<&'static str> {
    match cmie_kind {
        0 => Some("component"),
        1 => Some("module"),
        2 => Some("interface"),
        3 => Some("enum"),
        _ => None,
    }
}

/// Build a hover for a self-defining symbol.
fn format_symbol_hover(name: &str, kind: &str, scope: &str) -> Option<Hover> {
    let mut lines = vec![format!("`{}` — {}", name, kind)];
    if !scope.is_empty() {
        lines.push(format!("scope: `{}`", scope));
    }
    format_markdown_hover(&lines)
}

/// Format a single index entry as a human-readable line.
fn format_entry_line(entry: &IndexEntry) -> String {
    let span_info = format!("[{}:{}]", entry.span.0, entry.span.1);
    format!("{}  {}", entry.name, span_info)
}

// ── Markdown formatting ──

fn format_markdown(header: &str, lines: &[String]) -> MarkupContent {
    let mut value = header.to_string();
    for line in lines {
        value.push('\n');
        value.push_str(line);
    }
    MarkupContent {
        kind: MarkupKind::Markdown,
        value,
    }
}

fn format_markdown_hover(lines: &[String]) -> Option<Hover> {
    if lines.is_empty() {
        return None;
    }
    let value = lines.join("\n");
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    })
}

// Use-statement path helpers are in crate::util::usechk.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::LapperEntry;
    use crate::state::RpcSemSymbols;
    use ropey::Rope;
    use std::sync::{Arc, Mutex};
    use tower_lsp::lsp_types::{
        HoverParams, Position, TextDocumentIdentifier, TextDocumentPositionParams,
    };

    // ── Markdown formatting (pure functions) ──

    #[test]
    fn entry_line_formats_correctly() {
        let entry = IndexEntry {
            uri: Url::parse("file:///test.mc").unwrap(),
            span: (10, 20),
            name: "helper_chip".into(),
        };
        let line = format_entry_line(&entry);
        assert!(line.contains("helper_chip"), "line: {line}");
        assert!(line.contains("[10:20]"), "line: {line}");
    }

    #[test]
    fn markdown_header_and_lines() {
        let header = "### Definitions in helper.mc";
        let lines: Vec<String> = vec!["- helper_chip [0:20]".into()];
        let content = format_markdown(header, &lines);
        assert_eq!(content.kind, MarkupKind::Markdown);
        assert!(content.value.contains(header));
        assert!(content.value.contains("helper_chip"));
    }

    #[test]
    fn markdown_hover_empty_lines_returns_none() {
        let result = format_markdown_hover(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn markdown_hover_with_lines() {
        let lines = vec!["line 1".to_string(), "line 2".to_string()];
        let hover = format_markdown_hover(&lines).unwrap();
        match &hover.contents {
            HoverContents::Markup(mc) => {
                assert!(mc.value.contains("line 1"));
                assert!(mc.value.contains("line 2"));
            }
            _ => panic!("expected Markup"),
        }
    }

    #[test]
    fn symbol_hover_format() {
        let hover = format_symbol_hover("helper_chip", "component/module", "global").unwrap();
        match &hover.contents {
            HoverContents::Markup(mc) => {
                assert!(mc.value.contains("helper_chip"));
                assert!(mc.value.contains("component/module"));
            }
            _ => panic!("expected Markup"),
        }
    }

    #[test]
    fn def_text_from_rope_extracts_full_trimmed_line() {
        // Any byte offset inside a definition line returns the user's whole
        // declaration text — the pin's `io` statement here, not just `NRST`.
        let source = "module main\n{\n    io 7 = NRST, \"NRST\"\n}";
        let rope = Rope::from_str(source);
        let offset = byte_offset(source, "io 7 = NRST", 0).unwrap();
        let line = def_text_from_rope(&rope, offset).unwrap();
        assert_eq!(line, "io 7 = NRST, \"NRST\"");
    }

    #[test]
    fn def_text_from_rope_handles_byte_offset_inside_token() {
        // Offset lands mid-name (`NRST`) — still the whole definition line.
        let source = "component MCU\n{\n    io 7 = NRST, \"NRST\"\n}";
        let rope = Rope::from_str(source);
        let offset = byte_offset(source, "NRST", 0).unwrap() + 2;
        let line = def_text_from_rope(&rope, offset).unwrap();
        assert_eq!(line, "io 7 = NRST, \"NRST\"");
    }

    #[test]
    fn def_text_from_rope_follows_multiline_component_header() {
        // A multi-line `component CAP(...)` must include the full parameter
        // list, not just the truncated `component CAP(` opening line.
        let source = "component CAP(\n    cap::UV.CAP,\n    volt::UV.VOLT\n)\n{\n    name = \"Capacitor\"\n}";
        let rope = Rope::from_str(source);
        let offset = byte_offset(source, "component CAP", 0).unwrap() + 10;
        let text = def_text_from_rope(&rope, offset).unwrap();
        assert!(text.contains("component CAP("), "got: {text}");
        assert!(text.contains("cap::UV.CAP"), "got: {text}");
        assert!(text.contains("volt::UV.VOLT"), "got: {text}");
        // Stops once the header's brackets balance — the body is not included.
        assert!(!text.contains("Capacitor"), "got: {text}");
    }

    // ── Symbol hover ──

    fn state_with_lapper(
        lapper_entries: Vec<(u8, usize, usize, u32, &str)>,
    ) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        let source = "component main                \n";
        state.insert_document(uri.clone(), Rope::from_str(source), 1);
        let lapper: Vec<LapperEntry> = lapper_entries
            .into_iter()
            .map(|(kind, start, stop, id, scope)| LapperEntry {
                kind,
                start,
                stop,
                id,
                scope: scope.into(),
                file: "file:///test.mc".into(),
            })
            .collect();
        let symbols = RpcSemSymbols {
            lapper,
            ..Default::default()
        };
        state
            .symbols
            .sem_symbols
            .insert(uri.clone(), Arc::new(Mutex::new(symbols)));
        (state, uri)
    }

    #[test]
    fn class_def_hover_shows_kind() {
        // "component main" — "main" from byte 10 to 14
        let (state, uri) = state_with_lapper(vec![(0, 10, 14, 0, "")]);
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(0, 12),
            },
            work_done_progress_params: Default::default(),
        };
        let hover = resolve(&state, &params).unwrap();
        match &hover.contents {
            HoverContents::Markup(mc) => {
                assert!(
                    mc.value.contains("component/module"),
                    "expected component/module label, got: {}",
                    mc.value
                );
            }
            _ => panic!("expected Markup"),
        }
    }

    #[test]
    fn empty_lapper_returns_none() {
        let (state, uri) = state_with_lapper(vec![]);
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(0, 0),
            },
            work_done_progress_params: Default::default(),
        };
        let result = resolve(&state, &params);
        assert!(result.is_none(), "expected None for empty lapper");
    }

    #[test]
    fn out_of_bounds_position_returns_none() {
        let (state, uri) = state_with_lapper(vec![(0, 0, 10, 0, "")]);
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(100, 0),
            },
            work_done_progress_params: Default::default(),
        };
        let result = resolve(&state, &params);
        assert!(result.is_none(), "expected None for out-of-bounds");
    }

    #[test]
    fn pinref_hover_shows_full_def_line_via_refdefmap() {
        // `uC.ADC{P,N}` (PinIfaceRef) must resolve through the RefDefMap to
        // its def and LEAD with the def's full source line (`io [16, 17, 21]
        // = ADC::ADC.DIFF(Receiver)`) — the pin's original definition, not a
        // bare def name nor a bare `— → pin`.
        use crate::rpc::{LapperEntry, RefDefEntryData, RefDefMapData};
        use crate::state::RpcSemSymbols;
        use std::sync::{Arc, Mutex};

        let source = "module main\n{\n    MIC{P,N} -> uC.ADC{P,N}\n    MCU uC\n}\nio [16, 17, 21] = ADC::ADC.DIFF(Receiver)\n";
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///test.mc").unwrap();
        state.insert_document(uri.clone(), Rope::from_str(source), 1);

        // Ref: `uC.ADC{P,N}` → PinIfaceRef(15) id=7; def: `ADC` label → PinIfaceDef(14).
        let ref_start = byte_offset(source, "uC.ADC{P,N}", 0).unwrap();
        let ref_end = ref_start + "uC.ADC{P,N}".len();
        let def_start = byte_offset(source, "ADC::ADC.DIFF", 0).unwrap();
        let def_end = def_start + 3; // `ADC` label only

        let lapper = vec![LapperEntry {
            kind: 15,
            id: 7,
            start: ref_start,
            stop: ref_end,
            scope: "main".into(),
            file: "file:///test.mc".into(),
        }];
        let ref_def_map = RefDefMapData {
            entries: vec![RefDefEntryData {
                ref_kind: 15,
                ref_id: 7,
                file_id: 0,
                def_span: [def_start as u32, def_end as u32],
                def_kind: 14,
                container_id: 0,
                cmie_kind: 255,
                def_name: "ADC".into(),
            }],
            files: vec!["file:///test.mc".to_string()],
            containers: vec!["".to_string()],
            func_names: vec![],
            kind_names: vec![],
            result_id: 0,
            index: std::sync::OnceLock::new(),
            kind_map: std::sync::OnceLock::new(),
        };
        let symbols = RpcSemSymbols {
            lapper,
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

        // Hover on the middle of `uC.ADC{P,N}`.
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: pos_at(source, ref_start + 5),
            },
            work_done_progress_params: Default::default(),
        };
        let hover = resolve(&state, &params).unwrap();
        match &hover.contents {
            HoverContents::Markup(mc) => {
                assert!(
                    mc.value.contains("(pin)"),
                    "expected ref kind label, got: {}",
                    mc.value
                );
                // No reference arrows anywhere in the tooltip.
                assert!(
                    !mc.value.contains('→'),
                    "expected no → arrows in tooltip, got: {}",
                    mc.value
                );
                assert!(
                    mc.value.contains("ADC"),
                    "expected def name in tooltip, got: {}",
                    mc.value
                );
                // ★ The definition leads the tooltip as a full source line —
                // the pin's original `io` statement, not just the name `ADC`.
                assert!(
                    mc.value
                        .starts_with("```\nio [16, 17, 21] = ADC::ADC.DIFF(Receiver)\n```"),
                    "expected the full def line to lead the tooltip, got: {}",
                    mc.value
                );
                assert!(
                    mc.value.contains("test.mc"),
                    "expected def file in tooltip, got: {}",
                    mc.value
                );
            }
            _ => panic!("expected Markup"),
        }
    }

    fn byte_offset(source: &str, needle: &str, nth: usize) -> Option<usize> {
        source.match_indices(needle).nth(nth).map(|(i, _)| i)
    }

    fn pos_at(source: &str, offset: usize) -> Position {
        let rope = Rope::from_str(source);
        crate::common::position::offset_to_position(offset, &rope).unwrap_or(Position::new(0, 0))
    }

    // ── show.* subject extraction (hover grounding) ──

    /// Same shape as `state_with_lapper`, plus a pinned project index so the
    /// ClassDef→CMIE disambiguation branch has data to read.
    fn state_with_index(
        lapper_entries: Vec<(u8, usize, usize, u32, &str)>,
        index: crate::index::snapshot::ProjectIndex,
    ) -> (WorkspaceState, Url) {
        let (mut state, uri) = state_with_lapper(lapper_entries);
        state.project.index = crate::index::worker::IndexWorkerHandle::with_snapshot(index);
        (state, uri)
    }

    fn subject_of(state: &WorkspaceState, uri: &Url, line: u32, col: u32) -> Option<Subject> {
        hover_subject(
            state,
            uri,
            crate::common::position::position_to_offset(
                Position::new(line, col),
                &state.document_rope(uri).unwrap(),
            )
            .unwrap(),
        )
    }

    #[test]
    fn class_def_subject_disambiguates_via_index() {
        // "component main" with an index that classifies `main` as Interface:
        // the subject must be `interface`, not the generic ClassDef label.
        let (state, uri) = {
            let mut idx = crate::index::snapshot::ProjectIndex::new();
            idx.add(
                crate::index::snapshot::IndexKind::Interface,
                crate::index::snapshot::IndexEntry {
                    uri: Url::parse("file:///test.mc").unwrap(),
                    span: (0, 14),
                    name: "main".into(),
                },
            );
            state_with_index(vec![(0, 10, 14, 0, "")], idx)
        };
        let subject = subject_of(&state, &uri, 0, 12).expect("interface subject");
        assert_eq!(subject.kind, "interface");
        assert_eq!(subject.name, "main");
    }

    #[test]
    fn class_def_without_index_entry_has_no_subject() {
        // Empty index (lib file, index not built yet): no subject — the local
        // hover stands alone rather than guessing component vs module.
        let (state, uri) = state_with_index(
            vec![(0, 10, 14, 0, "")],
            crate::index::snapshot::ProjectIndex::new(),
        );
        assert!(subject_of(&state, &uri, 0, 12).is_none());
    }

    #[test]
    fn enum_def_subject_is_enum() {
        // An EnumDef (kind 16) needs no index disambiguation.
        let (state, uri) = state_with_lapper(vec![(16, 10, 14, 0, "")]);
        let subject = subject_of(&state, &uri, 0, 12).expect("enum subject");
        assert_eq!(subject.kind, "enum");
        assert_eq!(subject.name, "main");
    }

    #[test]
    fn class_ref_subject_reads_cmie_and_def_name() {
        // A ClassRef (kind 1) resolves through the RefDefMap: cmie_kind=2
        // (interface) wins over the generic ref label, and mcc's captured
        // def name replaces the (possibly hint-fallback) word.
        let (state, uri) = state_with_lapper(vec![(1, 10, 14, 7, "")]);
        let map = crate::rpc::RefDefMapData {
            entries: vec![crate::rpc::RefDefEntryData {
                ref_kind: 1,
                ref_id: 7,
                file_id: 0,
                def_span: [0, 14],
                def_kind: 0,
                container_id: 0,
                cmie_kind: 2,
                def_name: "MY_IF".into(),
            }],
            files: vec!["file:///test.mc".into()],
            containers: vec![],
            func_names: vec![],
            kind_names: vec![],
            result_id: 0,
            index: std::sync::OnceLock::new(),
            kind_map: std::sync::OnceLock::new(),
        };
        {
            let cell = state.symbols.sem_symbols.get(&uri).unwrap();
            let mut guard = cell.lock().unwrap();
            guard.ref_def_map = Some(map);
        }
        let subject = subject_of(&state, &uri, 0, 12).expect("ref subject");
        assert_eq!(subject.kind, "interface");
        assert_eq!(subject.name, "MY_IF");
    }

    #[test]
    fn pin_ref_has_no_subject() {
        // Pins have no show.* target — no subject, whatever the def map says.
        let (state, uri) = state_with_lapper(vec![(11, 10, 14, 7, "")]);
        assert!(subject_of(&state, &uri, 0, 12).is_none());
    }
}
