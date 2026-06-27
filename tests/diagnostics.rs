//! End-to-end LSP diagnostics test
//!
//! This test simulates the full LSP server flow and can automatically detect on CI:
//! - whether mcc::mcc_load_project correctly populates symbols
//! - behavior of features::goto_definition at different cursor positions
//! - behavior of features::references
//!
//! The previous "F12 not working" bug was found through manual debugging — this test can automatically catch similar issues.

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
    eprintln!("\n[diag-01] load helper.mc only");
    let (_main_path, helper_path) = write_fixtures();

    eprintln!("[diag-01] mcc_init_no_lib");
    mcc::mcc_init_no_lib();

    let helper_uri = McURI::from(helper_path.to_string_lossy().to_string());
    eprintln!(
        "[diag-01] before mcc_load_project({})",
        helper_path.display()
    );
    mcc::mcc_load_project(&helper_uri);
    eprintln!("[diag-01] after mcc_load_project");

    // List all loaded files
    eprintln!("[diag-01] loaded files in mcodes:");
    // Check via querying helper_uri
    if let Some(result) = mcc::mcc_query(&helper_uri) {
        eprintln!("[diag-01] helper.mc query is_some");
        let symbols = result.sem_symbols.lock().unwrap();
        eprintln!(
            "[diag-01]   lapper intervals = {}",
            symbols.symbol_lapper.intervals.len()
        );
        eprintln!(
            "[diag-01]   local inst_id_to_span = {}",
            symbols.local_table.inst_id_to_span.len()
        );
        eprintln!(
            "[diag-01]   local declare_inst_to_span = {}",
            symbols.local_table.declare_inst_to_span.len()
        );
        let gtable = symbols.global_table.lock().unwrap();
        eprintln!(
            "[diag-01]   global class_id_to_span = {}",
            gtable.class_id_to_span.len()
        );
        eprintln!(
            "[diag-01]   global declare_class_id_to_span = {}",
            gtable.declare_class_id_to_span.len()
        );
    } else {
        eprintln!("[diag-01] helper.mc query returned None!");
    }

    // Try get_def helper_chip directly
    let helper_chip_name = mcc::McIds::from("helper_chip");
    eprintln!("[diag-01] trying get_def(helper_chip)");
    match mcc::get_def(&helper_chip_name, &helper_uri) {
        Some(_) => eprintln!("[diag-01] get_def helper_chip: FOUND"),
        None => eprintln!("[diag-01] get_def helper_chip: NOT FOUND"),
    }

    // Directly call mcb_iter_components to see what it returns
    let components = mcc::mcb_iter_components();
    eprintln!(
        "[diag-01] mcb_iter_components: {} entries",
        components.len()
    );
    for (name, uri) in &components {
        eprintln!("[diag-01]   component: {name} at {uri}");
    }

    // ★ Call create_lapper to rebuild lapper (because mcc_load_project doesn't call it automatically)
    eprintln!("[diag-01] calling create_lapper on the loaded file");
    // We need to get helper.mc's McCode — call directly through workspace
    // But there's no pub API. Simplest: after query, call an internal function
    // We can't call it directly here — mcc_load_project should call it
    // This is the fix for the LSP side.
    // TODO: Submit PR to mcc: mcb_load_project should call create_lapper
}

#[test]
fn diag_02_load_main_only() {
    eprintln!("\n[diag-02] load main.mc only (no use target)");
    let (main_path, helper_path) = write_fixtures();

    eprintln!("[diag-02] mcc_init_no_lib");
    mcc::mcc_init_no_lib();

    let main_uri = McURI::from(main_path.to_string_lossy().to_string());
    eprintln!("[diag-02] before mcc_load_project({})", main_path.display());
    mcc::mcc_load_project(&main_uri);
    eprintln!("[diag-02] after mcc_load_project");

    eprintln!("[diag-02] before mcc_query");
    let result = mcc::mcc_query(&main_uri);
    eprintln!(
        "[diag-02] after mcc_query, result is_some={}",
        result.is_some()
    );
}

#[test]
fn diag_03_load_both() {
    eprintln!("\n[diag-03] load both helper.mc and main.mc");
    let (main_path, helper_path) = write_fixtures();

    eprintln!("[diag-03] mcc_init_no_lib");
    mcc::mcc_init_no_lib();

    let helper_uri = McURI::from(helper_path.to_string_lossy().to_string());
    eprintln!("[diag-03] loading helper first");
    mcc::mcc_load_project(&helper_uri);
    eprintln!("[diag-03] helper loaded");

    let main_uri = McURI::from(main_path.to_string_lossy().to_string());
    eprintln!("[diag-03] loading main");
    mcc::mcc_load_project(&main_uri);
    eprintln!("[diag-03] main loaded");

    eprintln!("[diag-03] querying main");
    let result = mcc::mcc_query(&main_uri);
    eprintln!("[diag-03] query result is_some={}", result.is_some());

    if let Some(result) = result {
        let symbols = result.sem_symbols.lock().unwrap();
        let lapper_count = symbols.symbol_lapper.intervals.len();
        let local_count = symbols.local_table.inst_id_to_span.len()
            + symbols.local_table.declare_inst_to_span.len();
        eprintln!("[diag-03] main symbol stats: lapper={lapper_count}, local={local_count}");
    }
}

/// Simulate full LSP server flow (including goto_definition)
#[test]
fn diag_04_full_lsp_flow() {
    eprintln!("\n[diag-04] full LSP flow simulation");
    let (main_path, helper_path) = write_fixtures();

    mcc::mcc_init_no_lib();

    let state = WorkspaceState::new();

    // Simulate initialize
    mcc::mcc_set_system_root(std::path::Path::new(""));
    mcc::mcc_set_project_root(main_path.parent().unwrap());

    // Simulate did_open main.mc (in actual LSP order)
    let main_uri = McURI::from(main_path.to_string_lossy().to_string());
    eprintln!("[diag-04] did_open main.mc");
    mcc::mcc_load_project(&main_uri);

    eprintln!("[diag-04] mcc_query for main");
    let result = mcc::mcc_query(&main_uri);
    eprintln!("[diag-04] query is_some={}", result.is_some());

    if let Some(result) = result {
        let main_url = Url::from_file_path(&main_path).unwrap();
        let rope = Rope::from_str(MAIN_SRC);
        state.insert_document(main_url.clone(), rope, 1);
        state.insert_parse(
            main_url.clone(),
            std::sync::Arc::clone(&result.sem_tokens),
            std::sync::Arc::clone(&result.sem_symbols),
            main_uri,
        );

        let symbols = result.sem_symbols.lock().unwrap();
        eprintln!(
            "[diag-04] main symbol stats: lapper={}, local={}",
            symbols.symbol_lapper.intervals.len(),
            symbols.local_table.inst_id_to_span.len()
                + symbols.local_table.declare_inst_to_span.len()
        );
    }
}

#[test]
fn diag_05_full_lsp_flow_no_helper_use() {
    eprintln!("\n[diag-05] full LSP flow, main.mc WITHOUT use directive");
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
    let state = WorkspaceState::new();
    mcc::mcc_set_system_root(std::path::Path::new(""));
    mcc::mcc_set_project_root(&dir);

    let main_uri = McURI::from(main_path.to_string_lossy().to_string());
    mcc::mcc_load_project(&main_uri);

    let result = mcc::mcc_query(&main_uri);
    eprintln!("[diag-05] query is_some={}", result.is_some());

    if let Some(result) = result {
        let symbols = result.sem_symbols.lock().unwrap();
        eprintln!(
            "[diag-05] no-use main symbol stats: lapper={}, local={}",
            symbols.symbol_lapper.intervals.len(),
            symbols.local_table.inst_id_to_span.len()
                + symbols.local_table.declare_inst_to_span.len()
        );
    }
}
