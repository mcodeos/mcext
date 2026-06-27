//! Integration test for cross-file scenarios
//!
//! Tests that `features::goto_definition` / `features::references` don't panic
//! in cross-file situations, and URI handling is correct.
//!
//! **Note**: This test does not call `mcc_load_from_string` (to avoid triggering mcast C library
//! state pollution). Cross-file real logic tests are verified end-to-end in VSCode.
//!
//! Here we only verify LSP layer logic: position → offset → symbol lookup → response,
//! running through the entire flow with fake state (empty parse).

use mcodels::features::{goto_definition, references};
use mcodels::WorkspaceState;
use ropey::Rope;
use tower_lsp::lsp_types::{Position, Url};

const MAIN_SRC: &str = "component main {\n    pins = [\n        io 1 = IN\n    ]\n}\n";
const HELPER_SRC: &str = "component helper_chip {\n    pins = [io 1 = IN_A]\n}\n";

/// Construct a state with two documents but **no parse**
fn setup() -> (WorkspaceState, Url, Url) {
    let state = WorkspaceState::new();
    let main_url = Url::parse("file:///test/main.mc").unwrap();
    let helper_url = Url::parse("file:///test/helper.mc").unwrap();
    state.insert_document(main_url.clone(), Rope::from_str(MAIN_SRC), 1);
    state.insert_document(helper_url.clone(), Rope::from_str(HELPER_SRC), 1);
    (state, main_url, helper_url)
}

#[test]
fn state_has_both_documents() {
    let (state, main_url, helper_url) = setup();
    assert!(state.document_rope(&main_url).is_some());
    assert!(state.document_rope(&helper_url).is_some());
    assert_ne!(main_url, helper_url);
}

#[test]
fn goto_definition_does_not_panic_on_any_position() {
    let (state, main_url, helper_url) = setup();
    // No panic at any position (all return None without parse)
    for line in 0..10 {
        for col in 0..40 {
            let _ = goto_definition::resolve(&state, &main_url, Position::new(line, col));
            let _ = goto_definition::resolve(&state, &helper_url, Position::new(line, col));
        }
    }
}

#[test]
fn references_does_not_panic_on_any_position() {
    let (state, main_url, helper_url) = setup();
    for line in 0..10 {
        for col in 0..40 {
            let _ = references::resolve(&state, &main_url, Position::new(line, col), true);
            let _ = references::resolve(&state, &helper_url, Position::new(line, col), false);
        }
    }
}

#[test]
fn goto_definition_returns_none_without_parse() {
    let (state, main_url, _helper_url) = setup();
    // No parse result, all resolve return None
    let result = goto_definition::resolve(&state, &main_url, Position::new(0, 5));
    assert!(result.is_none());
}

#[test]
fn references_returns_none_without_parse() {
    let (state, main_url, _helper_url) = setup();
    let result = references::resolve(&state, &main_url, Position::new(0, 5), true);
    assert!(result.is_none());
}
