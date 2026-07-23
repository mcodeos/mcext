//! Cross-file E2E: loads data/crossfile/main.mc using real mcc API,
//! verifies LSP features work with cross-file + pin definitions.
//!
//! Coverage:
//! - P1 Cross-file: goto_definition from main.mc's helper_chip reference jumps to helper.mc
//! - P4.1 Pin completion: completion::resolve returns non-empty results in `pins = [` context
//! - P0 Basic: semantic_tokens / diagnostics / references don't panic
//!
//! Run: `cargo test --test crossfile_e2e -- --nocapture`
//!
//! Note: This test triggers the mcast C library (complementary to tests/crossfile.rs's
//! "no parse" fake-state test, which doesn't call the C library).
//!
//! TODO: this test was written against the `mcc` crate, which has been removed
//! from `mcext` dependencies ("all interactions via RPC"). The original
//! `mcc::mcc_load_project / mcc_query` and `state.insert_parse` flow is no longer
//! available here.
//!
//! To re-enable: re-add the `mcc` crate as a dependency, or rewrite to drive
//! parsing via the RPC pipeline (see `tests/integration.rs`).
//!
//! The original test bodies are preserved below as a reference, but are gated
//! behind a never-enabled cfg so they do not break the build.

#[cfg(any())]
mod original {
    use mcc::{mcc_load_project, McURI};
    use mcodels::features::{completion, goto_definition, references, semantic_tokens};
    use mcodels::WorkspaceState;
    use ropey::Rope;
    use std::path::PathBuf;
    use tower_lsp::lsp_types::{
        CompletionContext as LspCompletionContext, CompletionParams, CompletionTriggerKind,
        PartialResultParams, Position, TextDocumentIdentifier, TextDocumentPositionParams, Url,
        WorkDoneProgressParams,
    };

    fn fixture_uri(name: &str) -> Url {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("data/crossfile");
        path.push(name);
        Url::from_file_path(path).expect("fixture path")
    }

    /// Load file into LSP state and trigger mcc parsing
    fn load_into_state(state: &WorkspaceState, url: &Url, name: &str) {
        let text =
            std::fs::read_to_string(url.to_file_path().expect("url->path")).expect("read fixture");

        state.insert_document(url.clone(), Rope::from_str(&text), 1);

        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("data/crossfile");
        path.push(name);
        let mc_uri = McURI::from(path.to_string_lossy().as_ref());
        mcc_load_project(&mc_uri);

        if let Some(parsed) = mcc::mcc_query(&mc_uri) {
            state.insert_parse(
                url.clone(),
                std::sync::Arc::clone(&parsed.sem_tokens),
                std::sync::Arc::clone(&parsed.sem_symbols),
                mc_uri,
            );
        }
    }

    #[test]
    fn crossfile_load_does_not_panic() {
        let state = WorkspaceState::new();
        let main_url = fixture_uri("main.mc");
        let helper_url = fixture_uri("helper.mc");
        load_into_state(&state, &main_url, "main.mc");
        load_into_state(&state, &helper_url, "helper.mc");
    }

    #[test]
    fn completion_in_pins_list_returns_items() {
        let state = WorkspaceState::new();
        let helper_url = fixture_uri("helper.mc");
        load_into_state(&state, &helper_url, "helper.mc");
        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: helper_url.clone() },
            position: Position::new(3, 12),
        };
        let result = completion::resolve(&state, &params);
        assert!(result.is_some());
    }

    #[test]
    fn completion_returns_keywords_at_general_position() {
        let state = WorkspaceState::new();
        let main_url = fixture_uri("main.mc");
        load_into_state(&state, &main_url, "main.mc");
        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: main_url.clone() },
            position: Position::new(5, 0),
        };
        let result = completion::resolve(&state, &params);
        assert!(result.is_some());
    }

    #[test]
    fn goto_definition_does_not_panic_at_instance_ref() {
        let state = WorkspaceState::new();
        let main_url = fixture_uri("main.mc");
        load_into_state(&state, &main_url, "main.mc");
        let _ = goto_definition::resolve(&state, &main_url, Position::new(5, 8));
    }

    #[test]
    fn references_does_not_panic_at_class_ref() {
        let state = WorkspaceState::new();
        let main_url = fixture_uri("main.mc");
        load_into_state(&state, &main_url, "main.mc");
        let _ = references::resolve(&state, &main_url, Position::new(5, 8), true);
    }

    #[test]
    fn semantic_tokens_compute_runs_on_real_parse() {
        let state = WorkspaceState::new();
        let helper_url = fixture_uri("helper.mc");
        load_into_state(&state, &helper_url, "helper.mc");
        let tokens = semantic_tokens::compute(&state, &helper_url);
        let _ = tokens;
    }

    /// Full completion params construction (kept for future P4.1 pin completion enhancements)
    #[allow(dead_code)]
    fn _completion_params(uri: &Url, pos: Position) -> CompletionParams {
        CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: pos,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: Some(LspCompletionContext {
                trigger_kind: CompletionTriggerKind::TRIGGER_CHARACTER,
                trigger_character: Some(".".to_string()),
            }),
        }
    }
}

#[test]
#[ignore = "placeholder; see module-level TODO for re-enable instructions"]
fn crossfile_e2e_placeholder() {
    // Intentionally empty — see module-level TODO.
}
