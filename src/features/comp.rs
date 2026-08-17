//! Completion — Auto-completion
//!
//! LSP entry points: `textDocument/completion` + `completionItem/resolve`
//!
//! Candidates are assembled from the P1-P4 layered name spaces (§5 of the
//! completion design doc): P1 = current func (params + local labels), P2 =
//! enclosing container (ports / labels / insts / funcs / pins / params /
//! attrs / buses), P3 = current file top-level CMIE, P4 = use-chain (local
//! degraded approximation: every indexed file). Syntax keywords supplement
//! the layers without participating in shadow. Cross-layer shadow follows
//! P1 > P2 > P3 > P4; same-layer dedup keys on `(name, kind)`.

use crate::common::position::position_to_offset;
use crate::features::context::{self, ContextKind};
use crate::index::snapshot::IndexKind;
use crate::rpc::{CompletionResponse as RpcCompletionResponse, LapperEntry, MccRpcClient};
use crate::state::WorkspaceState;
use ropey::Rope;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse,
    TextDocumentPositionParams, Url,
};

/// Max completion items to return (per §11, big layers are prefix-limited).
const MAX_ITEMS: usize = 50;

/// SymbolKind ordinals — must stay in sync with mcc `kind_names`
/// (see `features::context`).
const CLASS_DEF: u8 = 0;
const INST_DEF: u8 = 2;
const PORT_DEF: u8 = 4;
const LABEL_DEF: u8 = 6;
const FUNC_DEF: u8 = 8;
const PIN_ID_DEF: u8 = 10;
const PIN_NAME_DEF: u8 = 12;
const PIN_IFACE_DEF: u8 = 14;
const ENUM_DEF: u8 = 16;
const PARAM_DEF: u8 = 21;
const DEFINE_DEF: u8 = 22;
const ATTR_DEF: u8 = 23;
const BUS_DEF: u8 = 25;
const UNKNOWN_DEF: u8 = 27;

/// mcode syntax keywords (snippets). These supplement the layered spaces
/// only (§4.3 step 6) — they neither shadow symbols nor are shadowed.
const KEYWORDS: &[(&str, &str, &str)] = &[
    (
        "component",
        "Declare component",
        "component ${1:Name} {\n    pins = []\n}",
    ),
    (
        "interface",
        "Declare interface",
        "interface ${1:Name} {\n    pins = []\n}",
    ),
    (
        "enum",
        "Declare enum",
        "enum ${1:Name} {\n    ${2:Value}\n}",
    ),
    (
        "module",
        "Declare module",
        "module ${1:Name} {\n    ${2}\n}",
    ),
    ("pins", "Pin list", "pins = [${1}]"),
    ("config", "Config block", "config {\n    ${1}\n}"),
    ("use", "Import module", "use ${1:module}"),
    (
        "function",
        "Function definition",
        "function ${1:name}() {\n    ${2}\n}",
    ),
    ("return", "Return value", "return ${1:value}"),
    ("if", "Conditional", "if ${1:condition} {\n    ${2}\n}"),
    ("else", "Else branch", "else {\n    ${1}\n}"),
];

/// One layered completion candidate (§5).
#[derive(Debug, Clone)]
struct Candidate {
    name: String,
    /// P1-P4 layer number; keywords use 99 (appended after all layers).
    layer: u8,
    item_kind: CompletionItemKind,
    /// Within-layer stable kind order (§6.3): port/label/inst < param <
    /// func < pin < class.
    kind_order: u8,
    detail: String,
    insert_text: Option<String>,
}

// ── §7.3 / §8.3: layered snapshot cache ──

/// Cache key: uri + P1/P2 scope + P4 index fingerprint (§7.7).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    uri: Url,
    /// `container|func` — the scope pair that determines which lapper
    /// entries feed P1/P2.
    scope: String,
    /// `(files, entries)` of the project index; a library load / unload
    /// changes P4 content and bumps the fingerprint.
    index: (usize, usize),
    /// Member-access root (`uC`, `this`, ...) so different member spaces
    /// never collide in the cache (§8.3).
    member_root: Option<String>,
}

/// One cached candidate set for a `(uri, scope)` pair.
#[derive(Debug, Clone)]
struct SnapshotEntry {
    /// P1-P6 candidates **before** prefix filtering (P5 = mcode system lib,
    /// P6 = Member layer — both only present on the RPC path).
    cands: Vec<Candidate>,
    /// Content epoch at collection time (§7.3). A reparse / library load
    /// bumps `SymbolCache::parse_revision`; a mismatch means re-collect.
    revision: u32,
    /// Document version the lapper reflected when collected (§7.6). When it
    /// lags the current document version the snapshot is stale, so
    /// `is_incomplete = true`.
    parse_version: i32,
    /// Layers truncated by the mcc per-layer cap (§8.5); empty on the local
    /// degrade path.
    truncated_layers: Vec<String>,
    /// true when `cands` came from the layered completion RPC (§8.1);
    /// false for the local P1-P4 degrade path (§8.4).
    rpc_sourced: bool,
}

/// §7.3: per-file layered completion snapshot cache.
///
/// Candidates are collected from the lapper + project index only when the
/// key changes or the parse revision advances; every keystroke in between
/// reuses the cached candidate set and only re-runs the pure-local prefix
/// filter (§7.2) — no RPC, no lapper re-walk.
#[derive(Debug, Default)]
pub(crate) struct CompletionCache {
    inner: std::sync::Mutex<std::collections::HashMap<CacheKey, SnapshotEntry>>,
}

impl CompletionCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(&self, key: &CacheKey) -> Option<SnapshotEntry> {
        self.inner.lock().ok()?.get(key).cloned()
    }

    fn insert(&self, key: CacheKey, entry: SnapshotEntry) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.insert(key, entry);
        }
    }

    /// §7.7: drop all snapshots for a closed document.
    pub(crate) fn remove_uri(&self, uri: &Url) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.retain(|k, _| &k.uri != uri);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }
}

/// Compute completion response from local data only (P1-P4 degrade path,
/// §8.4). The layered RPC path is [`resolve_with_rpc`].
pub fn resolve(
    state: &WorkspaceState,
    params: &TextDocumentPositionParams,
) -> Option<CompletionResponse> {
    let uri = &params.text_document.uri;
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(params.position, &rope)?;

    // Per-file semantic lapper (spans may lag the edited rope — collectors
    // clamp against the rope length).
    let lapper = match state.symbols.sem_symbols.get(uri) {
        Some(s) => match s.lock() {
            Ok(s) => s.lapper.clone(),
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };

    let ctx = context::detect(&rope, offset, &lapper);

    // Comments / strings: nothing to complete (§7.5).
    if ctx.suppressed.is_some() {
        return None;
    }

    // Member-access and use-path are separate spaces (§5.6 / §5.7); the sync
    // path has no local data for them. Member access goes through the RPC
    // path (resolve_with_rpc); use-path is a later stage.
    if matches!(ctx.kind, ContextKind::MemberAccess | ContextKind::UsePath) {
        return None;
    }

    // ── §7.3: layered snapshot cache ──
    // The candidate set depends on (uri, scope) and on P4 index content;
    // context and prefix always read the current rope (§2.2 / §7.6).
    let cache_key = cache_key(state, uri, &ctx, None);
    let revision = state.symbols.parse_revision.load(Ordering::Relaxed);
    let doc_version = state.document_version(uri).unwrap_or(-1);
    let parse_version = state
        .symbols
        .parse_versions
        .get(uri)
        .map(|v| *v)
        .unwrap_or(-1);

    match state.completion.get(&cache_key) {
        Some(entry) if entry.revision == revision => {
            // Fresh candidate set — reuse it; only prefix filtering runs
            // locally from here (§7.2).
            build_response(&entry, &ctx.prefix, ctx.kind, doc_version)
        }
        _ => {
            // Miss or stale content epoch — collect the local P1-P4 degrade
            // snapshot (§8.4).
            let mut cands: Vec<Candidate> = Vec::new();
            if let Some(func_scope) = ctx.func_scope.as_deref() {
                collect_p1(&lapper, &rope, func_scope, &mut cands);
            }
            if !ctx.container_scope.is_empty() {
                collect_p2(&lapper, &rope, &ctx.container_scope, &mut cands);
            }
            collect_p3(&lapper, &rope, &mut cands);
            collect_p4(state, uri, &mut cands);
            let entry = SnapshotEntry {
                cands,
                revision,
                parse_version,
                truncated_layers: Vec::new(),
                rpc_sourced: false,
            };
            state.completion.insert(cache_key, entry.clone());
            build_response(&entry, &ctx.prefix, ctx.kind, doc_version)
        }
    }
}

/// Layered completion via the mcc completion RPC (§8.1), with a 300 ms
/// budget; on failure or timeout it degrades to the local P1-P4 path (§8.4).
///
/// Member-access contexts are answered entirely from the RPC `Member` layer
/// (§5.6) — the sync path has no local data for them, so on RPC failure they
/// yield `None`.
pub async fn resolve_with_rpc(
    state: &WorkspaceState,
    rpc: &MccRpcClient,
    params: &TextDocumentPositionParams,
) -> Option<CompletionResponse> {
    let uri = &params.text_document.uri;
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(params.position, &rope)?;

    let lapper = match state.symbols.sem_symbols.get(uri) {
        Some(s) => match s.lock() {
            Ok(s) => s.lapper.clone(),
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };

    let ctx = context::detect(&rope, offset, &lapper);

    // Comments / strings: nothing to complete (§7.5).
    if ctx.suppressed.is_some() {
        return None;
    }
    // Use-path completion is a later stage (not part of the layered RPC yet).
    if ctx.kind == ContextKind::UsePath {
        return None;
    }

    let cache_key = cache_key(state, uri, &ctx, ctx.member_root.clone());
    let revision = state.symbols.parse_revision.load(Ordering::Relaxed);
    let doc_version = state.document_version(uri).unwrap_or(-1);
    let parse_version = state
        .symbols
        .parse_versions
        .get(uri)
        .map(|v| *v)
        .unwrap_or(-1);

    // Fresh RPC snapshot → local prefix filter only, zero RPC per keystroke
    // (§7.2).
    if let Some(entry) = state.completion.get(&cache_key) {
        if entry.revision == revision {
            return build_response(&entry, &ctx.prefix, ctx.kind, doc_version);
        }
    }

    // Cache miss / stale epoch → layered RPC. The whole call (lock wait +
    // request) is budgeted at 300 ms so a busy mcc server never blocks user
    // input; the sync path below then degrades (§8.4).
    let rpc_result: Result<RpcCompletionResponse, String> = {
        let fut = async {
            let _guard = state.rpc_lock.lock().await;
            rpc.completion(
                uri.as_str(),
                offset,
                Some(&ctx.prefix),
                ctx.member_root.as_deref(),
            )
            .await
        };
        match tokio::time::timeout(Duration::from_millis(300), fut).await {
            Ok(res) => res.map_err(|e| e.to_string()),
            Err(_) => Err("timeout after 300 ms".to_string()),
        }
    };

    match rpc_result {
        Ok(resp) => {
            let (cands, truncated_layers) = rpc_candidates(&resp);
            let entry = SnapshotEntry {
                cands,
                revision,
                parse_version,
                truncated_layers,
                rpc_sourced: true,
            };
            state.completion.insert(cache_key, entry.clone());
            build_response(&entry, &ctx.prefix, ctx.kind, doc_version)
        }
        Err(e) => {
            tracing::debug!(
                "completion RPC unavailable ({}), falling back to local P1-P4",
                e
            );
            resolve(state, params)
        }
    }
}

/// Build the cache key for a context. `member_root` is `Some` only for
/// member-access positions, keeping `uC.` and `this.` snapshots apart.
fn cache_key(
    state: &WorkspaceState,
    uri: &Url,
    ctx: &context::CompletionContext,
    member_root: Option<String>,
) -> CacheKey {
    CacheKey {
        uri: uri.clone(),
        scope: format!(
            "{}|{}",
            ctx.container_scope,
            ctx.func_scope.as_deref().unwrap_or("")
        ),
        index: state.project.index.fingerprint(),
        member_root,
    }
}

/// Assemble the final `CompletionList` from a snapshot: keywords supplement,
/// dedup + shadow + prefix filter + sort (§6 / §7.2), truncation to
/// [`MAX_ITEMS`].
///
/// `is_incomplete` (§7.4): false only when a fresh RPC snapshot is in sync
/// with the document; a lagging parse, a truncated layer, or a local degrade
/// snapshot all signal `true`.
fn build_response(
    entry: &SnapshotEntry,
    prefix: &str,
    ctx_kind: ContextKind,
    doc_version: i32,
) -> Option<CompletionResponse> {
    let mut keywords: Vec<Candidate> = Vec::new();
    if matches!(
        ctx_kind,
        ContextKind::TopLevel | ContextKind::ContainerBody | ContextKind::FuncBody
    ) {
        collect_keywords(&mut keywords);
    }

    let items = assemble(entry.cands.clone(), keywords, prefix);
    if items.is_empty() {
        return None;
    }
    // Items are sorted; truncate the tail (lowest layers / weakest matches).
    let items = items.into_iter().take(MAX_ITEMS).collect::<Vec<_>>();
    let is_incomplete = doc_version != entry.parse_version
        || !entry.truncated_layers.is_empty()
        || !entry.rpc_sourced;

    Some(CompletionResponse::List(CompletionList {
        is_incomplete,
        items,
    }))
}

/// Convert a layered completion RPC response into local candidates (§8.1).
/// `P1`..`P5` map to layers 1-5, `Member` to layer 6. Returns
/// `(cands, truncated_layers)`.
fn rpc_candidates(resp: &RpcCompletionResponse) -> (Vec<Candidate>, Vec<String>) {
    let mut cands: Vec<Candidate> = Vec::new();
    for (layer_str, items) in &resp.layers {
        let layer = match layer_str.as_str() {
            "P1" => 1,
            "P2" => 2,
            "P3" => 3,
            "P4" => 4,
            "P5" => 5,
            "Member" => 6,
            _ => continue,
        };
        for it in items {
            let (item_kind, kind_order, label) = kind_meta(&it.kind);
            cands.push(Candidate {
                name: it.name.clone(),
                layer,
                item_kind,
                kind_order,
                detail: format!("{layer_str} · {label}"),
                insert_text: None,
            });
        }
    }
    (cands, resp.truncated_layers.clone())
}

/// Map an mcc `LookupSymbolKind` string (mcc `kind_names`) to an LSP item
/// kind, within-layer order, and display label. Ordering follows the local
/// collectors (§6.3): port/instance/label/param < func < pin/enum_value <
/// class/define.
fn kind_meta(kind: &str) -> (CompletionItemKind, u8, &'static str) {
    match kind {
        "port" => (CompletionItemKind::PROPERTY, 0, "port"),
        "instance" => (CompletionItemKind::VALUE, 0, "instance"),
        "label" => (CompletionItemKind::VARIABLE, 1, "label"),
        "param" => (CompletionItemKind::VARIABLE, 1, "param"),
        "function" => (CompletionItemKind::FUNCTION, 2, "function"),
        "pin" => (CompletionItemKind::ENUM_MEMBER, 3, "pin"),
        "enum_value" => (CompletionItemKind::ENUM_MEMBER, 3, "enum value"),
        "enum" => (CompletionItemKind::ENUM, 0, "enum"),
        "interface" => (CompletionItemKind::INTERFACE, 1, "interface"),
        "module" => (CompletionItemKind::MODULE, 2, "module"),
        "component" => (CompletionItemKind::CLASS, 3, "component"),
        "define" => (CompletionItemKind::CONSTANT, 4, "define"),
        "role" => (CompletionItemKind::PROPERTY, 4, "role"),
        _ => (CompletionItemKind::PROPERTY, 4, "symbol"),
    }
}

// ── P1: func space (§5.1) ──

fn collect_p1(lapper: &[LapperEntry], rope: &Rope, func_scope: &str, out: &mut Vec<Candidate>) {
    for e in lapper {
        if e.kind != PARAM_DEF && e.kind != LABEL_DEF && e.kind != UNKNOWN_DEF {
            continue;
        }
        if e.scope != func_scope {
            continue;
        }
        let name = clamped_text(rope, e);
        if name.is_empty() {
            continue;
        }
        let (kind_order, label) = if e.kind == PARAM_DEF {
            (0, "param")
        } else {
            (1, "label")
        };
        out.push(Candidate {
            name,
            layer: 1,
            item_kind: CompletionItemKind::VARIABLE,
            kind_order,
            detail: format!("P1 · {label}"),
            insert_text: None,
        });
    }
}

// ── P2: container space (§5.2) ──

fn collect_p2(lapper: &[LapperEntry], rope: &Rope, container: &str, out: &mut Vec<Candidate>) {
    // Port/label co-registration (§6.2): a LabelDef registered at the exact
    // span of a PortDef is the same port — keep only the PortDef.
    let port_spans: HashSet<(usize, usize)> = lapper
        .iter()
        .filter(|e| e.kind == PORT_DEF && e.scope == container)
        .map(|e| (e.start, e.stop))
        .collect();

    let func_prefix = format!("{container}.");
    for e in lapper {
        if e.kind == FUNC_DEF {
            // FuncDef scope is `container.func`.
            if !e.scope.starts_with(&func_prefix) {
                continue;
            }
            let name = clamped_text(rope, e);
            if name.is_empty() {
                continue;
            }
            out.push(Candidate {
                name,
                layer: 2,
                item_kind: CompletionItemKind::FUNCTION,
                kind_order: 2,
                detail: "P2 · func".into(),
                insert_text: None,
            });
            continue;
        }

        let meta = match e.kind {
            PORT_DEF => Some((0, "port", CompletionItemKind::PROPERTY)),
            LABEL_DEF => Some((1, "label", CompletionItemKind::VARIABLE)),
            INST_DEF => Some((0, "instance", CompletionItemKind::VALUE)),
            BUS_DEF => Some((0, "bus", CompletionItemKind::PROPERTY)),
            PARAM_DEF => Some((1, "param", CompletionItemKind::VARIABLE)),
            PIN_ID_DEF | PIN_NAME_DEF | PIN_IFACE_DEF => {
                Some((3, "pin", CompletionItemKind::ENUM_MEMBER))
            }
            ATTR_DEF => Some((4, "attr", CompletionItemKind::PROPERTY)),
            _ => None,
        };
        let Some((kind_order, label, item_kind)) = meta else {
            continue;
        };
        if e.scope != container {
            continue;
        }
        // Same-span label as a port: already covered by the PortDef.
        if e.kind == LABEL_DEF && port_spans.contains(&(e.start, e.stop)) {
            continue;
        }
        let name = clamped_text(rope, e);
        if name.is_empty() {
            continue;
        }
        out.push(Candidate {
            name,
            layer: 2,
            item_kind,
            kind_order,
            detail: format!("P2 · {label}"),
            insert_text: None,
        });
    }
}

// ── P3: current file space (§5.3) ──

fn collect_p3(lapper: &[LapperEntry], rope: &Rope, out: &mut Vec<Candidate>) {
    for e in lapper {
        let (kind_order, label, name) = match e.kind {
            ENUM_DEF => match context::header_def(rope, e.start) {
                Some((_kw, n)) => (0, "enum", n),
                None => continue,
            },
            DEFINE_DEF => (4, "define", clamped_text(rope, e)),
            CLASS_DEF => match context::header_def(rope, e.start) {
                Some((kw, n)) => {
                    let (order, label) = match kw.as_str() {
                        "interface" => (1, "interface"),
                        "module" => (2, "module"),
                        _ => (3, "component"),
                    };
                    (order, label, n)
                }
                None => continue,
            },
            _ => continue,
        };
        if name.is_empty() {
            continue;
        }
        let item_kind = match label {
            "interface" => CompletionItemKind::INTERFACE,
            "module" => CompletionItemKind::MODULE,
            "enum" => CompletionItemKind::ENUM,
            "define" => CompletionItemKind::CONSTANT,
            _ => CompletionItemKind::CLASS,
        };
        out.push(Candidate {
            name,
            layer: 3,
            item_kind,
            kind_order,
            detail: format!("P3 · {label}"),
            insert_text: None,
        });
    }
}

// ── P4: use-chain space (§5.4, degraded) ──

fn collect_p4(state: &WorkspaceState, current_uri: &Url, out: &mut Vec<Candidate>) {
    let snap = state.project.index.snapshot();
    let kinds: [(IndexKind, &str, u8, CompletionItemKind); 4] = [
        (
            IndexKind::Component,
            "component",
            0,
            CompletionItemKind::CLASS,
        ),
        (
            IndexKind::Interface,
            "interface",
            1,
            CompletionItemKind::INTERFACE,
        ),
        (IndexKind::Module, "module", 2, CompletionItemKind::MODULE),
        (IndexKind::Enum, "enum", 3, CompletionItemKind::ENUM),
    ];
    for (kind, label, order, item_kind) in kinds {
        let _ = label;
        for entry in snap.iter_kind(kind) {
            if &entry.uri == current_uri {
                // Current-file CMIE already live in P3.
                continue;
            }
            out.push(Candidate {
                name: entry.name.clone(),
                layer: 4,
                item_kind,
                kind_order: order,
                detail: format!("P4 · use chain — {}", entry.uri.as_str()),
                insert_text: None,
            });
        }
    }
}

// ── Keywords ──

fn collect_keywords(out: &mut Vec<Candidate>) {
    for &(label, detail, insert) in KEYWORDS {
        out.push(Candidate {
            name: label.to_string(),
            layer: 99,
            item_kind: CompletionItemKind::KEYWORD,
            kind_order: 0,
            detail: detail.to_string(),
            insert_text: Some(insert.to_string()),
        });
    }
}

// ── Assembly: dedup, shadow, prefix filter, sort (§6 / §7.2) ──

/// Build final completion items: per-layer `(name, kind)` dedup, cross-layer
/// shadow by name (P1 > P2 > P3 > P4), prefix filtering (§7.2), and sorting
/// (layer asc → prefix-match quality → kind order → name).
fn assemble(cands: Vec<Candidate>, keywords: Vec<Candidate>, prefix: &str) -> Vec<CompletionItem> {
    let mut claimed: HashSet<String> = HashSet::new();
    let mut picked: Vec<(Candidate, u8)> = Vec::new(); // (candidate, quality)

    // P1..P6: P5 = mcode system lib, P6 = Member layer (both only on the RPC
    // path). Keywords (99) are appended after all layers.
    for layer in 1..=6 {
        let mut seen: HashSet<(String, u8)> = HashSet::new();
        // Names claimed by this layer only shadow *lower* layers (§6.1);
        // names claimed in earlier layers shadow the whole layer (§6.3 rule 4).
        let mut layer_claimed: HashSet<String> = HashSet::new();
        for c in cands.iter().filter(|c| c.layer == layer) {
            // Same-layer dedup by (name, kind): `enum CAP` + `component CAP`
            // coexist (§6.2); cross-layer shadow drops the rest.
            if claimed.contains(&c.name) || !seen.insert((c.name.clone(), c.kind_order)) {
                continue;
            }
            layer_claimed.insert(c.name.clone());
            if let Some(q) = match_quality(&c.name, prefix) {
                picked.push((c.clone(), q));
            }
        }
        claimed.extend(layer_claimed);
    }
    // Keywords supplement the layers without participating in shadow.
    for c in keywords {
        if let Some(q) = match_quality(&c.name, prefix) {
            picked.push((c, q));
        }
    }

    picked.sort_by(|(a, qa), (b, qb)| {
        a.layer
            .cmp(&b.layer)
            .then(qa.cmp(qb))
            .then(a.kind_order.cmp(&b.kind_order))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.name.cmp(&b.name))
    });

    picked
        .into_iter()
        .map(|(c, q)| {
            // sortText encodes layer + match quality so the client sorts
            // identically (§6.3).
            let sort_text = format!("{:02}{:02}{}", c.layer, q, c.name);
            CompletionItem {
                label: c.name,
                kind: Some(c.item_kind),
                detail: Some(c.detail),
                insert_text: c.insert_text,
                sort_text: Some(sort_text),
                ..Default::default()
            }
        })
        .collect()
}

/// §7.2: exact prefix > case-insensitive prefix > substring; `None` = no
/// match. Empty prefix matches everything.
fn match_quality(name: &str, prefix: &str) -> Option<u8> {
    if prefix.is_empty() {
        return Some(0);
    }
    if name.starts_with(prefix) {
        return Some(0);
    }
    let nl = name.to_lowercase();
    let pl = prefix.to_lowercase();
    if nl.starts_with(&pl) {
        return Some(1);
    }
    nl.contains(&pl).then_some(2)
}

/// Text at a lapper span, clamped against the current rope (spans may lag
/// the edited document).
fn clamped_text(rope: &Rope, e: &LapperEntry) -> String {
    let start = e.start.min(rope.len_bytes());
    let stop = e.stop.min(rope.len_bytes());
    if start >= stop {
        return String::new();
    }
    rope.byte_slice(start..stop).to_string()
}

/// Resolve additional info for a completion item (no-op for now).
pub fn resolve_item(item: CompletionItem) -> CompletionItem {
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::CompletionLayerItem;
    use std::sync::{Arc, Mutex};

    fn lapper_entry(kind: u8, start: usize, stop: usize, scope: &str) -> LapperEntry {
        LapperEntry {
            kind,
            start,
            stop,
            id: 0,
            scope: scope.to_string(),
            file: String::new(),
        }
    }

    /// `module main { ... }` — ClassDef `main` at 7..11 (name token), ports
    /// `I2C0`/`V3V3` with scope "main", func `i2c` at 23..26 with a param
    /// label `a` (scope "main.i2c"), plus an enum and a define.
    fn sample_lapper() -> Vec<LapperEntry> {
        vec![
            lapper_entry(CLASS_DEF, 7, 11, ""),          // module main
            lapper_entry(PORT_DEF, 24, 28, "main"),      // I2C0
            lapper_entry(PORT_DEF, 34, 38, "main"),      // V3V3
            lapper_entry(LABEL_DEF, 24, 28, "main"),     // I2C0 (co-registered)
            lapper_entry(FUNC_DEF, 23, 26, "main.i2c"),  // func i2c
            lapper_entry(LABEL_DEF, 30, 31, "main.i2c"), // param a
            lapper_entry(ENUM_DEF, 60, 63, ""),          // enum PKG
            lapper_entry(CLASS_DEF, 80, 83, ""),         // component CAP
            lapper_entry(ENUM_DEF, 92, 95, ""),          // enum CAP (same name)
            lapper_entry(DEFINE_DEF, 100, 103, ""),      // define VDD
        ]
    }

    #[test]
    fn p1_shadows_p2() {
        // func param `a` (P1) shadows any P2 `a`.
        let cands = vec![
            Candidate {
                name: "a".into(),
                layer: 2,
                item_kind: CompletionItemKind::PROPERTY,
                kind_order: 0,
                detail: "P2 · port".into(),
                insert_text: None,
            },
            Candidate {
                name: "a".into(),
                layer: 1,
                item_kind: CompletionItemKind::VARIABLE,
                kind_order: 0,
                detail: "P1 · param".into(),
                insert_text: None,
            },
        ];
        let items = assemble(cands, vec![], "a");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].detail.as_deref(), Some("P1 · param"));
    }

    #[test]
    fn same_layer_enum_and_component_coexist() {
        // §6.2: `enum CAP` and `component CAP` in the same layer both show.
        // Lapper spans = name-token offsets in the source below.
        let src = "module main {\n    I2C0\n    V3V3\n}\nenum PKG {\n    SOP8\n}\ncomponent CAP {\n}\nenum CAP {\n}\ndefine VDD = 5V\n";
        let lapper = vec![
            lapper_entry(CLASS_DEF, 7, 11, ""),   // module main
            lapper_entry(ENUM_DEF, 39, 42, ""),   // enum PKG
            lapper_entry(CLASS_DEF, 66, 69, ""),  // component CAP
            lapper_entry(ENUM_DEF, 79, 82, ""),   // enum CAP
            lapper_entry(DEFINE_DEF, 94, 97, ""), // define VDD
        ];
        let mut cands = Vec::new();
        collect_p3(&lapper, &ropey::Rope::from_str(src), &mut cands);
        let items = assemble(cands, vec![], "CAP");
        let names: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(names, vec!["CAP", "CAP"]);
    }

    #[test]
    fn p3_shadows_p4() {
        // Same name in P3 (current file) shadows P4 (index/use chain).
        let p3 = Candidate {
            name: "RES".into(),
            layer: 3,
            item_kind: CompletionItemKind::CLASS,
            kind_order: 3,
            detail: "P3 · component".into(),
            insert_text: None,
        };
        let p4 = Candidate {
            name: "RES".into(),
            layer: 4,
            item_kind: CompletionItemKind::CLASS,
            kind_order: 0,
            detail: "P4 · use chain".into(),
            insert_text: None,
        };
        let items = assemble(vec![p3, p4], vec![], "R");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].detail.as_deref(), Some("P3 · component"));
    }

    #[test]
    fn port_label_co_registration_keeps_port() {
        // §6.2: same-span PortDef + LabelDef → only the PortDef shows.
        let rope = ropey::Rope::from_str("module main {\n    I2C0\n    V3V3\n}\n");
        let lapper = vec![
            lapper_entry(CLASS_DEF, 7, 11, ""),
            lapper_entry(PORT_DEF, 24, 28, "main"),
            lapper_entry(LABEL_DEF, 24, 28, "main"),
        ];
        let mut cands = Vec::new();
        collect_p2(&lapper, &rope, "main", &mut cands);
        let items = assemble(cands, vec![], "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].detail.as_deref(), Some("P2 · port"));
    }

    #[test]
    fn func_body_collects_p1_and_p2() {
        // Inside func `i2c`: P1 param `a` first, then P2 members.
        let rope =
            ropey::Rope::from_str("module main {\n    func i2c(a) {\n        I2C0 -> \n    }\n}\n");
        let mut cands = Vec::new();
        collect_p1(&sample_lapper(), &rope, "main.i2c", &mut cands);
        collect_p2(&sample_lapper(), &rope, "main", &mut cands);
        let items = assemble(cands, vec![], "");
        let details: Vec<&str> = items.iter().map(|i| i.detail.as_deref().unwrap()).collect();
        assert_eq!(
            details,
            vec!["P1 · label", "P2 · port", "P2 · port", "P2 · func"]
        );
    }

    #[test]
    fn prefix_filter_quality_order() {
        // Exact prefix before case-insensitive prefix before substring.
        let mk = |name: &str, layer: u8| Candidate {
            name: name.into(),
            layer,
            item_kind: CompletionItemKind::CLASS,
            kind_order: 0,
            detail: String::new(),
            insert_text: None,
        };
        let cands = vec![mk("resx", 4), mk("RES", 4), mk("myres", 4)];
        // Prefix "RES": case-consistent exact match for RES (q0), prefix
        // ignoring case for resx (q1), substring for myres (q2).
        let items = assemble(cands, vec![], "RES");
        let names: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(names, vec!["RES", "resx", "myres"]);
    }

    #[test]
    fn suppressed_context_returns_none() {
        // Comment position → no completion.
        let state: WorkspaceState = WorkspaceState::new();
        // resolve() needs a document rope; exercised indirectly via
        // context::detect in integration — here assert the guard logic by
        // detecting a comment position.
        let rope = ropey::Rope::from_str("// hello");
        let ctx = context::detect(&rope, 7, &[]);
        assert_eq!(ctx.suppressed, Some(context::SuppressReason::Comment));
        // The server path maps suppression to None inside resolve(); the
        // detection itself is covered above.
        let _ = state;
    }

    #[test]
    fn keywords_supplement_without_shadowing() {
        let kw = Candidate {
            name: "if".into(),
            layer: 99,
            item_kind: CompletionItemKind::KEYWORD,
            kind_order: 0,
            detail: "Conditional".into(),
            insert_text: Some("if ${1:condition} {\n    ${2}\n}".into()),
        };
        let sym = Candidate {
            name: "if".into(),
            layer: 3,
            item_kind: CompletionItemKind::CLASS,
            kind_order: 3,
            detail: "P3 · component".into(),
            insert_text: None,
        };
        let items = assemble(vec![sym], vec![kw], "");
        assert_eq!(items.len(), 2); // keyword and symbol both show
        assert_eq!(items[0].detail.as_deref(), Some("P3 · component"));
        assert_eq!(items[1].kind, Some(CompletionItemKind::KEYWORD));
    }

    // ── S3: snapshot cache (§7.3) + is_incomplete (§7.4 / §7.6) ──

    #[test]
    fn cache_hit_returns_collected_snapshot() {
        let cache = CompletionCache::new();
        let uri = Url::parse("file:///t.mc").unwrap();
        let key = CacheKey {
            uri: uri.clone(),
            scope: "main|".into(),
            index: (1, 3),
            member_root: None,
        };
        let entry = SnapshotEntry {
            cands: vec![Candidate {
                name: "a".into(),
                layer: 1,
                item_kind: CompletionItemKind::VARIABLE,
                kind_order: 0,
                detail: "P1 · param".into(),
                insert_text: None,
            }],
            revision: 0,
            parse_version: 1,
            truncated_layers: Vec::new(),
            rpc_sourced: true,
        };
        assert!(cache.get(&key).is_none());
        cache.insert(key.clone(), entry.clone());
        let got = cache.get(&key).expect("snapshot cached");
        assert_eq!(got.revision, 0);
        assert_eq!(got.parse_version, 1);
        assert_eq!(got.cands.len(), 1);
        // §7.7: removing the document drops its snapshots.
        cache.remove_uri(&uri);
        assert!(cache.get(&key).is_none());
    }

    /// Build a `WorkspaceState` with one parsed document (lapper present).
    /// `doc_version` / `parse_version` drive the staleness signal.
    fn parsed_state(src: &str, doc_version: i32, parse_version: i32) -> (WorkspaceState, Url) {
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///t.mc").unwrap();
        state.insert_document(uri.clone(), ropey::Rope::from_str(src), doc_version);
        let lapper = vec![
            lapper_entry(CLASS_DEF, 7, 11, ""),          // module main
            lapper_entry(FUNC_DEF, 23, 26, "main.i2c"),  // func i2c
            lapper_entry(LABEL_DEF, 27, 28, "main.i2c"), // param a
        ];
        state.symbols.sem_symbols.insert(
            uri.clone(),
            Arc::new(Mutex::new(crate::state::RpcSemSymbols {
                lapper,
                local_declares: vec![],
                local_references: vec![],
                global_declares: vec![],
                global_references: vec![],
                ref_def_map: None,
            })),
        );
        state
            .symbols
            .parse_versions
            .insert(uri.clone(), parse_version);
        (state, uri)
    }

    fn completion_params(uri: &Url, line: u32, character: u32) -> TextDocumentPositionParams {
        TextDocumentPositionParams {
            text_document: tower_lsp::lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            position: tower_lsp::lsp_types::Position::new(line, character),
        }
    }

    #[test]
    fn fresh_local_snapshot_degrades_with_candidates() {
        // The sync path is the local P1-P4 degrade path (§8.4): even with the
        // parse in sync with the document, it lacks P5 and cannot filter the
        // use chain — so it always signals is_incomplete (§7.4). Candidates
        // are still offered from the lapper.
        let src = "module main {\n    func i2c(a) {\n        a -> \n    }\n}\n";
        let (state, uri) = parsed_state(src, 1, 1);
        let resp = resolve(&state, &completion_params(&uri, 2, 12)).expect("completion");
        let CompletionResponse::List(list) = resp else {
            panic!("expected a list");
        };
        assert!(
            list.is_incomplete,
            "local degrade must signal is_incomplete"
        );
        // P1 func param `a` is offered.
        assert!(list
            .items
            .iter()
            .any(|i| i.detail.as_deref() == Some("P1 · label")));

        // A second keystroke in the same scope hits the cache (no re-collect).
        let resp2 = resolve(&state, &completion_params(&uri, 2, 12)).expect("completion");
        let CompletionResponse::List(list2) = resp2 else {
            panic!("expected a list");
        };
        assert_eq!(list.items.len(), list2.items.len());
    }

    #[test]
    fn stale_parse_marks_incomplete() {
        // Document advanced to v2 but the parse still reflects v1 (§7.6).
        let src = "module main {\n    func i2c(a) {\n        a -> \n    }\n}\n";
        let (state, uri) = parsed_state(src, 2, 1);
        let resp = resolve(&state, &completion_params(&uri, 2, 12)).expect("completion");
        let CompletionResponse::List(list) = resp else {
            panic!("expected a list");
        };
        assert!(list.is_incomplete, "lagger parse must signal is_incomplete");
        // Candidates are still offered from the lagging lapper.
        assert!(list
            .items
            .iter()
            .any(|i| i.detail.as_deref() == Some("P1 · label")));
    }

    #[test]
    fn reparse_bumps_revision_and_recollects() {
        // After a reparse (parse_revision bumped, lapper refreshed) the old
        // snapshot is invalidated even though the key is unchanged.
        let src = "module main {\n    func i2c(a) {\n        a -> \n    }\n}\n";
        let (state, uri) = parsed_state(src, 2, 2);
        // First request caches under revision 0.
        let _ = resolve(&state, &completion_params(&uri, 2, 12));
        assert_eq!(state.completion.len(), 1);

        // A reparse with the same doc version bumps the content epoch.
        state.symbols.parse_revision.fetch_add(1, Ordering::Relaxed);
        state.symbols.parse_versions.insert(uri.clone(), 2);
        let resp = resolve(&state, &completion_params(&uri, 2, 12)).expect("completion");
        let CompletionResponse::List(list) = resp else {
            panic!("expected a list");
        };
        // Local degrade path → always incomplete (§7.4), even after reparse.
        assert!(list.is_incomplete);
        assert!(list
            .items
            .iter()
            .any(|i| i.detail.as_deref() == Some("P1 · label")));
        // The stale entry was replaced, not accumulated.
        assert_eq!(state.completion.len(), 1);
    }

    #[test]
    fn member_access_and_use_path_return_none() {
        // Member access / use path are separate spaces — the sync path has
        // no local data for them (§5.6 / §5.7); member access goes through
        // resolve_with_rpc.
        let src = "uC.PA";
        let (state, uri) = parsed_state(src, 1, 1);
        assert!(resolve(&state, &completion_params(&uri, 0, 5)).is_none());
        let src2 = "use ./po";
        let (state2, uri2) = parsed_state(src2, 1, 1);
        assert!(resolve(&state2, &completion_params(&uri2, 0, 8)).is_none());
    }

    // ── S4: layered RPC conversion (§8.1) ──

    /// Build an `RpcCompletionResponse` from a `(layer, items)` map.
    fn rpc_resp(layers: &[(&str, &[(&str, &str)])], truncated: &[&str]) -> RpcCompletionResponse {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        for (layer, items) in layers {
            let v: Vec<CompletionLayerItem> = items
                .iter()
                .map(|(name, kind)| CompletionLayerItem {
                    name: name.to_string(),
                    kind: kind.to_string(),
                    scope: String::new(),
                    uri: String::new(),
                    span: crate::rpc::CompletionSpan::default(),
                })
                .collect();
            map.insert(layer.to_string(), v);
        }
        RpcCompletionResponse {
            scope_path: "US513.i2c".to_string(),
            layers: map,
            truncated_layers: truncated.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn rpc_candidates_maps_layers_and_kinds() {
        let resp = rpc_resp(
            &[
                (
                    "P2",
                    &[("uC", "instance"), ("I2C0", "port"), ("i2c", "function")][..],
                ),
                ("P5", &[("CAP", "component")][..]),
                ("Member", &[("VDD", "pin"), ("power", "function")][..]),
            ],
            &["P5"],
        );
        let (cands, truncated) = rpc_candidates(&resp);
        assert_eq!(truncated, vec!["P5"]);
        assert_eq!(cands.len(), 6);
        // P5 candidates land in layer 5, Member in layer 6.
        let p5 = cands.iter().find(|c| c.name == "CAP").unwrap();
        assert_eq!(p5.layer, 5);
        assert_eq!(p5.item_kind, CompletionItemKind::CLASS);
        let mem = cands.iter().find(|c| c.name == "VDD").unwrap();
        assert_eq!(mem.layer, 6);
        assert_eq!(mem.item_kind, CompletionItemKind::ENUM_MEMBER);
        assert_eq!(mem.detail, "Member · pin");
    }

    #[test]
    fn p5_and_member_layers_sort_after_p4() {
        // Final sort keeps the layer order P4 < P5 < Member.
        let cands = vec![
            Candidate {
                name: "VDD".into(),
                layer: 6,
                item_kind: CompletionItemKind::ENUM_MEMBER,
                kind_order: 3,
                detail: "Member · pin".into(),
                insert_text: None,
            },
            Candidate {
                name: "CAP".into(),
                layer: 5,
                item_kind: CompletionItemKind::CLASS,
                kind_order: 3,
                detail: "P5 · component".into(),
                insert_text: None,
            },
            Candidate {
                name: "RES".into(),
                layer: 4,
                item_kind: CompletionItemKind::CLASS,
                kind_order: 0,
                detail: "P4 · use chain".into(),
                insert_text: None,
            },
        ];
        let items = assemble(cands, vec![], "");
        let details: Vec<&str> = items.iter().map(|i| i.detail.as_deref().unwrap()).collect();
        assert_eq!(
            details,
            vec!["P4 · use chain", "P5 · component", "Member · pin"]
        );
    }

    #[test]
    fn p4_shadows_p5_same_name() {
        // §6.1: a use-chain name (P4) shadows the same name in the system
        // library (P5).
        let cands = vec![
            Candidate {
                name: "CAP".into(),
                layer: 5,
                item_kind: CompletionItemKind::CLASS,
                kind_order: 3,
                detail: "P5 · component".into(),
                insert_text: None,
            },
            Candidate {
                name: "CAP".into(),
                layer: 4,
                item_kind: CompletionItemKind::CLASS,
                kind_order: 0,
                detail: "P4 · use chain".into(),
                insert_text: None,
            },
        ];
        let items = assemble(cands, vec![], "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].detail.as_deref(), Some("P4 · use chain"));
    }

    #[test]
    fn member_roots_do_not_collide_in_cache() {
        // Different member roots (`uC` vs `this`) must key separate
        // snapshots (§8.3), so a `this.` request never reuses `uC`'s members.
        let state = WorkspaceState::new();
        let uri = Url::parse("file:///t.mc").unwrap();
        let ctx_uc = context::detect(&ropey::Rope::from_str("uC.PA"), 5, &[]);
        let ctx_this = context::detect(&ropey::Rope::from_str("this.VDD"), 8, &[]);
        let k1 = cache_key(&state, &uri, &ctx_uc, Some("uC".into()));
        let k2 = cache_key(&state, &uri, &ctx_this, Some("this".into()));
        assert_ne!(k1, k2);
        assert_eq!(k1.member_root.as_deref(), Some("uC"));
    }

    #[test]
    fn build_response_flags_truncation() {
        // A fresh RPC snapshot with a truncated layer → is_incomplete = true
        // (§8.5).
        let entry = SnapshotEntry {
            cands: vec![Candidate {
                name: "CAP".into(),
                layer: 5,
                item_kind: CompletionItemKind::CLASS,
                kind_order: 3,
                detail: "P5 · component".into(),
                insert_text: None,
            }],
            revision: 0,
            parse_version: 1,
            truncated_layers: vec!["P5".into()],
            rpc_sourced: true,
        };
        let resp = build_response(&entry, "CAP", ContextKind::ContainerBody, 1).expect("response");
        let CompletionResponse::List(list) = resp else {
            panic!("expected a list");
        };
        assert!(list.is_incomplete);
        assert_eq!(list.items.len(), 1);
    }
}
