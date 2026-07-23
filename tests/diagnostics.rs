//! End-to-end LSP diagnostics test
//!
//! This test simulates the full LSP server flow and can automatically detect on CI:
//! - whether mcc::mcc_load_project correctly populates symbols
//! - behavior of features::goto_definition at different cursor positions
//! - behavior of features::references
//!
//! The previous "F12 not working" bug was found through manual debugging — this test can automatically catch similar issues.
//!
//! TODO: this test was written against the `mcc` crate, which has been removed
//! from `mcext` dependencies ("all interactions via RPC"). The original
//! `mcc::mcc_load_project / mcc_query / mcc_set_*_root` and `state.insert_parse`
//! flow is no longer available here.
//!
//! To re-enable: re-add the `mcc` crate as a dependency, or rewrite to drive
//! parsing via the RPC pipeline (see `tests/integration.rs`).
//!
//! The original test bodies are preserved below as a reference, but are gated
//! behind a never-enabled cfg so they do not break the build.

#[cfg(any())]
mod original {
    use mcc::McURI;
    use mcodels::state::WorkspaceState;
    use ropey::Rope;
    use tower_lsp::lsp_types::Url;

    const HELPER_SRC: &str = "\
// helper.mc — defines helper_chip component
component helper_chip {
    pins = [
        io 1 = IN_A
        io 2 = IN_B
        io 3 = OUT
    ]
}
";

    const MAIN_SRC: &str = "\
// main.mc — uses helper_chip (defined in helper.mc)
use ./helper.mc

module main
{
    helper_chip hc1
    hc1.1 - SIG_OUT
}
";

    fn write_fixtures() -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir()
            .join("mcext_diag_test")
            .join("crossfile");
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.mc");
        let helper = dir.join("helper.mc");
        std::fs::write(&main, MAIN_SRC).unwrap();
        std::fs::write(&helper, HELPER_SRC).unwrap();
        (main, helper)
    }

    #[test]
    fn diag_01_load_helper_only() {
        let (_main_path, helper_path) = write_fixtures();
        mcc::mcc_init_no_lib();
        let helper_uri = McURI::from(helper_path.to_string_lossy().to_string());
        mcc::mcc_load_project(&helper_uri);
        let _ = mcc::mcc_query(&helper_uri);
    }

    #[test]
    fn diag_02_load_main_only() {
        let (main_path, _helper_path) = write_fixtures();
        mcc::mcc_init_no_lib();
        let main_uri = McURI::from(main_path.to_string_lossy().to_string());
        mcc::mcc_load_project(&main_uri);
        let _ = mcc::mcc_query(&main_uri);
    }

    #[test]
    fn diag_03_load_both() {
        let (main_path, helper_path) = write_fixtures();
        mcc::mcc_init_no_lib();
        let helper_uri = McURI::from(helper_path.to_string_lossy().to_string());
        mcc::mcc_load_project(&helper_uri);
        let main_uri = McURI::from(main_path.to_string_lossy().to_string());
        mcc::mcc_load_project(&main_uri);
        let _ = mcc::mcc_query(&main_uri);
    }

    #[test]
    fn diag_04_full_lsp_flow() {
        let (main_path, _helper_path) = write_fixtures();
        mcc::mcc_init_no_lib();
        let state = WorkspaceState::new();
        mcc::mcc_set_system_root(std::path::Path::new(""));
        mcc::mcc_set_project_root(main_path.parent().unwrap());
        let main_uri = McURI::from(main_path.to_string_lossy().to_string());
        mcc::mcc_load_project(&main_uri);
        if let Some(result) = mcc::mcc_query(&main_uri) {
            let main_url = Url::from_file_path(&main_path).unwrap();
            let rope = Rope::from_str(MAIN_SRC);
            state.insert_document(main_url.clone(), rope, 1);
            state.insert_parse(
                main_url.clone(),
                std::sync::Arc::clone(&result.sem_tokens),
                std::sync::Arc::clone(&result.sem_symbols),
                main_uri,
            );
        }
    }

    #[test]
    fn diag_05_full_lsp_flow_no_helper_use() {
        let dir = std::env::temp_dir().join("mcext_diag_test_no_use");
        std::fs::create_dir_all(&dir).unwrap();
        let main_path = dir.join("main.mc");
        let main_src = "\
module main
{
    // no helper_chip use; everything self-contained
}
";
        std::fs::write(&main_path, main_src).unwrap();
        mcc::mcc_init_no_lib();
        let _state = WorkspaceState::new();
        mcc::mcc_set_system_root(std::path::Path::new(""));
        mcc::mcc_set_project_root(&dir);
        let main_uri = McURI::from(main_path.to_string_lossy().to_string());
        mcc::mcc_load_project(&main_uri);
        let _ = mcc::mcc_query(&main_uri);
    }
}

#[test]
#[ignore = "placeholder; see module-level TODO for re-enable instructions"]
fn diagnostics_placeholder() {
    // Intentionally empty — see module-level TODO.
}
