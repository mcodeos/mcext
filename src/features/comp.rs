//! Completion — Auto-completion
//!
//! LSP entry points: `textDocument/completion` + `completionItem/resolve`
//!
//! Completion sources:
//! 1. **Keyword completion** - mcode syntax keywords (component, interface, pins, etc.)
//! 2. **Component name completion** - component names in current file and project
//! 3. **Module/Interface/Enum name completion** - project-level symbols
//!
//! Completion is triggered by the LSP protocol; VS Code requests automatically on user input.

use crate::common::position::position_to_offset;
use crate::state::WorkspaceState;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionResponse, InsertTextFormat,
    TextDocumentPositionParams, Url,
};

/// mcode syntax keywords
const KEYWORDS: &[(&str, &str, &str)] = &[
    // (keyword, detail, insert_text)
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
    ("pins", "Pin list", "pins = [${1}]"),
    ("config", "Config block", "config {\n    ${1}\n}"),
    ("property", "Property", "property ${1:name} = ${2:value}"),
    ("use", "Import module", "use ${1:module}"),
    ("meta", "Metadata", "meta {\n    ${1}\n}"),
    ("type", "Type definition", "type ${1:Name}"),
    (
        "function",
        "Function definition",
        "function ${1:name}() {\n    ${2}\n}",
    ),
    ("on", "Event handler", "on ${1:event} {\n    ${2}\n}"),
    ("return", "Return value", "return ${1:value}"),
    ("if", "Conditional", "if ${1:condition} {\n    ${2}\n}"),
    ("else", "Else branch", "else {\n    ${1}\n}"),
    ("for", "For loop", "for ${1:i} in ${2:range} {\n    ${3}\n}"),
    ("while", "While loop", "while ${1:condition} {\n    ${2}\n}"),
    ("var", "Variable declaration", "var ${1:name} = ${2:value}"),
    (
        "const",
        "Constant declaration",
        "const ${1:name} = ${2:value}",
    ),
    ("true", "Boolean true", "true"),
    ("false", "Boolean false", "false"),
    ("null", "Null value", "null"),
];

/// Completion context
#[derive(Debug)]
pub enum CompletionContext {
    /// Inside component/interface/enum declaration body
    Body,
    /// In pin definition
    Pins,
    /// In use statement
    Use,
    /// General context (falls back to keywords)
    General,
    /// Unable to determine context
    Unknown,
}

/// Analyze completion context based on position
fn analyze_context(rope: &ropey::Rope, offset: usize) -> CompletionContext {
    // Get current line text
    let line_idx = match rope.try_byte_to_line(offset) {
        Ok(l) => l,
        Err(_) => return CompletionContext::Unknown,
    };

    // Defensive: ensure line_idx is within bounds
    if line_idx >= rope.len_lines() {
        return CompletionContext::Unknown;
    }

    let line_text = rope.line(line_idx).to_string();
    let offset_in_line = offset.saturating_sub(rope.line_to_byte(line_idx));

    // Get current word - use character index consistently
    let line_chars: Vec<char> = line_text.chars().collect();
    let line_char_len = line_chars.len();
    let offset_in_line_chars = offset_in_line.min(line_char_len);

    let mut char_idx = offset_in_line_chars;
    while char_idx < line_char_len
        && (line_chars[char_idx].is_alphanumeric() || line_chars[char_idx] == '_')
    {
        char_idx += 1;
    }

    // Analyze text before current word
    let before_text: String = line_chars[..char_idx].iter().collect();
    let before_text = before_text.trim().to_string();

    // Check if in use statement
    if before_text.starts_with("use") || before_text.ends_with("use") {
        return CompletionContext::Use;
    }

    // Check if in pins definition
    if before_text.contains("pins") || before_text.ends_with('=') {
        // May be in pin list
        if before_text.contains('[') && !before_text.contains(']') {
            return CompletionContext::Pins;
        }
    }

    // Check if in declaration body (has braces)
    let char_offset = match rope.try_byte_to_char(offset) {
        Ok(c) => c,
        Err(_) => return CompletionContext::Unknown,
    };
    let text_before = rope.slice(..char_offset.min(rope.len_chars()));
    let brace_count = text_before.chars().filter(|c| *c == '{').count()
        - text_before.chars().filter(|c| *c == '}').count();
    if brace_count > 0 {
        return CompletionContext::Body;
    }

    CompletionContext::General
}

/// Compute completion response
pub fn resolve(
    state: &WorkspaceState,
    params: &TextDocumentPositionParams,
) -> Option<CompletionResponse> {
    let uri = &params.text_document.uri;
    let rope = state.document_rope(uri)?;
    let offset = position_to_offset(params.position, &rope)?;

    let context = analyze_context(&rope, offset);

    let mut items = Vec::new();

    // Return different completion candidates based on context
    match context {
        CompletionContext::Use => {
            // use statement: complete module names
            items.extend(resolve_module_names(state, uri));
        }
        CompletionContext::Pins => {
            // pins: complete pin names (if any)
            items.extend(resolve_pins(state, uri));
        }
        _ => {
            // Default: keywords + component names
            items.extend(resolve_keywords());
            items.extend(resolve_component_names(state, uri));
            items.extend(resolve_interface_names(state, uri));
            items.extend(resolve_enum_names(state, uri));
        }
    }

    if items.is_empty() {
        return None;
    }

    Some(CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items,
    }))
}

/// Keyword completion
fn resolve_keywords() -> Vec<CompletionItem> {
    KEYWORDS
        .iter()
        .map(|(label, detail, insert_text)| CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(detail.to_string()),
            insert_text: Some(insert_text.to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        })
        .collect()
}

/// Component name completion
fn resolve_component_names(state: &WorkspaceState, uri: &Url) -> Vec<CompletionItem> {
    let mut names = Vec::new();

    // Components in current file (extracted from global_declares)
    if let Some(symbols_ref) = state.sem_symbols.get(uri) {
        if let Ok(symbols) = symbols_ref.lock() {
            for decl in &symbols.global_declares {
                // Filter by file path
                if decl.uri.contains(uri.path()) || decl.uri.is_empty() {
                    names.push(CompletionItem {
                        label: decl.uri.clone(), // Note: id used as label placeholder
                        kind: Some(CompletionItemKind::CLASS),
                        detail: Some("Component".to_string()),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Components in project (from cached project_symbols)
    if let Ok(cache) = state.project_symbols.lock() {
        for comp in &cache.components {
            if comp.uri.contains(uri.path()) {
                continue; // Skip current file
            }
            if !names.iter().any(|i: &CompletionItem| i.label == comp.name) {
                names.push(CompletionItem {
                    label: comp.name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(format!("Component ({})", comp.uri)),
                    ..Default::default()
                });
            }
        }
    }

    names
}

/// Interface name completion
fn resolve_interface_names(state: &WorkspaceState, _uri: &Url) -> Vec<CompletionItem> {
    let mut names = Vec::new();

    if let Ok(cache) = state.project_symbols.lock() {
        for item in &cache.interfaces {
            names.push(CompletionItem {
                label: item.name.clone(),
                kind: Some(CompletionItemKind::INTERFACE),
                detail: Some(format!("Interface ({})", item.uri)),
                ..Default::default()
            });
        }
    }

    names
}

/// Enum name completion
fn resolve_enum_names(state: &WorkspaceState, _uri: &Url) -> Vec<CompletionItem> {
    let mut names = Vec::new();

    if let Ok(cache) = state.project_symbols.lock() {
        for item in &cache.enums {
            names.push(CompletionItem {
                label: item.name.clone(),
                kind: Some(CompletionItemKind::ENUM),
                detail: Some(format!("Enum ({})", item.uri)),
                ..Default::default()
            });
        }
    }

    names
}

/// Module name completion (for use statements)
fn resolve_module_names(state: &WorkspaceState, _uri: &Url) -> Vec<CompletionItem> {
    let mut names = Vec::new();

    if let Ok(cache) = state.project_symbols.lock() {
        for item in &cache.modules {
            names.push(CompletionItem {
                label: item.name.clone(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some(format!("Module ({})", item.uri)),
                insert_text: Some(item.name.clone()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }
    }

    names
}

/// Common signal pin names (snippet format, inserted directly as pin identifier)
const COMMON_PINS: &[(&str, &str)] = &[
    ("VDD", "Positive power"),
    ("VCC", "Positive power"),
    ("VSS", "Negative power"),
    ("GND", "Ground"),
    ("SCL", "I2C clock"),
    ("SDA", "I2C data"),
    ("TX", "UART transmit"),
    ("RX", "UART receive"),
    ("MOSI", "SPI master out slave in"),
    ("MISO", "SPI master in slave out"),
    ("SCLK", "SPI clock"),
    ("CS", "Chip select"),
    ("INT", "Interrupt"),
    ("IRQ", "Interrupt request"),
    ("RESET", "Reset"),
    ("EN", "Enable"),
    ("D+", "USB data positive"),
    ("D-", "USB data negative"),
    ("WP", "Write protect"),
    ("HOLD", "Hold"),
    ("CLK", "Clock"),
];

/// Pin completion — completes common signal names at `pins = [` declaration.
///
/// Current scope: only completes identifier positions inside `pins = [...]`; common signal names
/// (VDD/GND/SCL/SDA, etc.) are provided as snippet suggestions, users can continue typing custom names.
///
/// Future extension (>20 lines, requires mcc API): `instance.PIN` dot completion needs to find
/// DeclareInstance by the IDENT before cursor, then get `names_to_id.keys()` by its class name.
fn resolve_pins(_state: &WorkspaceState, _uri: &Url) -> Vec<CompletionItem> {
    COMMON_PINS
        .iter()
        .map(|(name, detail)| CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(detail.to_string()),
            insert_text: Some(name.to_string()),
            ..Default::default()
        })
        .collect()
}

/// Additional info for completionItem/resolve
pub fn resolve_item(item: CompletionItem) -> CompletionItem {
    // More detailed info can be added here
    // Currently returns the original item
    item
}

// Note: Tests disabled - require mcc server connection
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::state::WorkspaceState;
//     use mcc::McURI;
//     use ropey::Rope;
//     use std::sync::Arc;
//     use tower_lsp::lsp_types::Position;
//
//     fn fake_state(text: &str) -> (WorkspaceState, Url) {
//         let state = WorkspaceState::new();
//         let uri = Url::parse("file:///test.mc").unwrap();
//         state.insert_document(uri.clone(), Rope::from_str(text), 1);
//
//         let mc_uri = McURI::from("/test.mc");
//         mcc::mcc_load_from_string(&mc_uri, text);
//
//         if let Some(result) = mcc::mcc_query(&mc_uri) {
//             state.insert_parse(
//                 uri.clone(),
//                 Arc::clone(&result.sem_tokens),
//                 Arc::clone(&result.sem_symbols),
//                 mc_uri,
//             );
//         }
//
//         (state, uri)
//     }
//
//     #[test]
//     fn resolve_returns_completions() {
//         let (state, uri) = fake_state("component X { pins = [1] }\n");
//         let params = TextDocumentPositionParams {
//             text_document: tower_lsp::lsp_types::TextDocumentIdentifier { uri },
//             position: Position::new(0, 0),
//         };
//         let result = resolve(&state, &params);
//         assert!(result.is_some());
//     }
//
//     #[test]
//     fn keywords_contain_common_keywords() {
//         let (state, uri) = fake_state("comp");
//         let params = TextDocumentPositionParams {
//             text_document: tower_lsp::lsp_types::TextDocumentIdentifier { uri },
//             position: Position::new(0, 4),
//         };
//         let result = resolve(&state, &params);
//         assert!(result.is_some());
//         let items = match result.unwrap() {
//             CompletionResponse::List(list) => list.items,
//             CompletionResponse::Array(arr) => arr,
//         };
//         // 应该包含 component
//         assert!(items.iter().any(|i| i.label == "component"));
//     }
//
//     #[test]
//     fn analyze_context_in_component_body() {
//         let rope = Rope::from_str("component X {\n    pins");
