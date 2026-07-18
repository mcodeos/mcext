//! Completion — Auto-completion
//!
//! LSP entry points: `textDocument/completion` + `completionItem/resolve`
//!
//! Completion sources (in priority order):
//! 1. Local symbols from lapper (ports, labels, instances — context-aware)
//! 2. Project index (components, modules, interfaces, enums)
//! 3. Keywords (syntax)

use crate::common::position::position_to_offset;
use crate::index::snapshot::IndexKind;
use crate::state::WorkspaceState;
use ropey::Rope;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, InsertTextFormat,
    TextDocumentPositionParams, Url,
};

/// Max completion items to return.
const MAX_ITEMS: usize = 50;

/// mcode syntax keywords.
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

/// Compute completion response.
pub fn resolve(
    state: &WorkspaceState,
    params: &TextDocumentPositionParams,
) -> Option<CompletionResponse> {
    let uri = &params.text_document.uri;
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(params.position, &rope)?;

    let mut items: Vec<CompletionItem> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Local symbols from the lapper (context-aware)
    collect_local_symbols(state, uri, &rope, offset, &mut items, &mut seen);

    // 2. Project index (components, modules, interfaces, enums)
    collect_index_symbols(state, &mut items, &mut seen);

    // 3. Keywords
    collect_keywords(&mut items, &mut seen);

    if items.is_empty() {
        return None;
    }
    items.truncate(MAX_ITEMS);

    Some(CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items,
    }))
}

// ── Source 1: local symbols from lapper ──

fn collect_local_symbols(
    state: &WorkspaceState,
    uri: &Url,
    rope: &Rope,
    offset: usize,
    items: &mut Vec<CompletionItem>,
    seen: &mut std::collections::HashSet<String>,
) {
    let symbols_ref = match state.sem_symbols.get(uri) {
        Some(s) => s,
        None => return,
    };
    let symbols = match symbols_ref.lock() {
        Ok(s) => s,
        Err(_) => return,
    };

    // Deduplicate by name (only first occurrence per name)
    let mut added: std::collections::HashSet<String> = std::collections::HashSet::new();

    for entry in &symbols.lapper {
        let name = rope.byte_slice(entry.start..entry.stop).to_string();
        // Skip: empty names, already added, anonymous (@-prefixed)
        if name.is_empty() || name.starts_with('@') || added.contains(&name) {
            continue;
        }
        added.insert(name.clone());

        let (kind, detail) = match entry.kind.as_str() {
            "port_def" => (CompletionItemKind::PROPERTY, "Port"),
            "label_def" => (CompletionItemKind::VARIABLE, "Label"),
            "instance_def" | "declare_instance" => (CompletionItemKind::VALUE, "Instance"),
            "function_def" => (CompletionItemKind::FUNCTION, "Function"),
            "class_def" | "class_definition" => (CompletionItemKind::CLASS, "Class def"),
            "class_ref" | "declare_class" => continue, // skip refs
            "instance_ref" | "label_ref" | "function_ref" | "pin_name_ref" | "interface_ref"
            | "enum_value_ref" | "enum_class_ref" => continue, // skip refs
            "pin_name_def" => (CompletionItemKind::ENUM_MEMBER, "Pin"),
            "enum_value_def" => (CompletionItemKind::ENUM_MEMBER, "Enum value"),
            "enum_class_def" => (CompletionItemKind::ENUM, "Enum"),
            "define_def" => (CompletionItemKind::CONSTANT, "Define"),
            "role_def" => (CompletionItemKind::INTERFACE, "Role"),
            _ => continue,
        };

        seen.insert(name.clone());
        items.push(CompletionItem {
            label: name,
            kind: Some(kind),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    }
}

// ── Source 2: project index ──

fn collect_index_symbols(
    state: &WorkspaceState,
    items: &mut Vec<CompletionItem>,
    seen: &mut std::collections::HashSet<String>,
) {
    let snap = state.index.snapshot();

    for (kind, kind_label, item_kind) in &[
        (IndexKind::Component, "Component", CompletionItemKind::CLASS),
        (IndexKind::Module, "Module", CompletionItemKind::MODULE),
        (
            IndexKind::Interface,
            "Interface",
            CompletionItemKind::INTERFACE,
        ),
        (IndexKind::Enum, "Enum", CompletionItemKind::ENUM),
    ] {
        // Use the snapshot's by_name entries
        for entry in snap.iter_kind(*kind) {
            if seen.contains(&entry.name) {
                continue;
            }
            seen.insert(entry.name.clone());
            items.push(CompletionItem {
                label: entry.name.clone(),
                kind: Some(*item_kind),
                detail: Some(format!("{} — {}", kind_label, entry.uri)),
                ..Default::default()
            });
        }
    }
}

// ── Source 3: keywords ──

fn collect_keywords(items: &mut Vec<CompletionItem>, seen: &mut std::collections::HashSet<String>) {
    for &(label, detail, insert_text) in KEYWORDS {
        if seen.contains(label) {
            continue;
        }
        seen.insert(label.to_string());
        items.push(CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(detail.to_string()),
            insert_text: Some(insert_text.to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        });
    }
}

/// Resolve additional info for a completion item (no-op for now).
pub fn resolve_item(item: CompletionItem) -> CompletionItem {
    item
}
