//! Integration tests with a real mcc server subprocess.
//!
//! Prerequisites: `mcc` binary at `../mcc/target/debug/mcc` or on PATH.
//! These tests verify the RPC pipeline: start server → load fixture → call features.

use std::sync::Arc;
use std::time::Duration;

async fn wait_connected(server: &mcodels::mccsrv::MccServer) -> bool {
    for _ in 0..50 {
        if server.is_connected() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)
}

#[tokio::test]
async fn mcc_server_starts_and_responds() {
    let mut server = mcodels::mccsrv::MccServer::new();
    server.start().await.expect("mcc server should start");
    assert!(
        wait_connected(&server).await,
        "server should connect within 10s"
    );
    server.stop().await.ok();
}

#[tokio::test]
async fn sem_returns_tokens_and_symbols() {
    let mut server = mcodels::mccsrv::MccServer::new();
    server.start().await.expect("mcc server should start");
    assert!(wait_connected(&server).await);

    let path = fixture_path("helper.mc");
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;
    let _ = client.set_project_root(&path).await;
    let _ = client.add_file("helper.mc").await;

    let content = std::fs::read_to_string(&path).expect("fixture exists");
    let sem = client.sem("helper.mc", Some(&content)).await;
    assert!(sem.is_ok(), "sem RPC failed: {:?}", sem.err());
    let sem = sem.unwrap();

    assert!(!sem.tokens.is_empty(), "should have raw tokens");
    assert!(!sem.symbols.lapper.is_empty(), "should have lapper entries");
    assert!(
        sem.symbols.lapper.iter().any(|e| e.kind == "class_def"),
        "helper_chip should produce a class_def lapper entry"
    );

    server.stop().await.ok();
}

#[tokio::test]
async fn diagnostics_no_error_for_valid_file() {
    let mut server = mcodels::mccsrv::MccServer::new();
    server.start().await.expect("mcc server should start");
    assert!(wait_connected(&server).await);

    let path = fixture_path("helper.mc");
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;
    let _ = client.set_project_root(&path).await;
    let _ = client.add_file("helper.mc").await;

    let diags = client.diagnostics("helper.mc").await;
    assert!(diags.is_ok(), "diagnostics RPC failed: {:?}", diags.err());
    // A valid fixture should have zero diagnostics
    assert!(
        diags.unwrap().diagnostics.is_empty(),
        "helper.mc should have no diagnostics"
    );

    server.stop().await.ok();
}

#[tokio::test]
async fn sem_tokens_compute_from_real_data() {
    let mut server = mcodels::mccsrv::MccServer::new();
    server.start().await.expect("mcc server should start");
    assert!(wait_connected(&server).await);

    let path = fixture_path("helper.mc");
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;
    let _ = client.set_project_root(&path).await;
    let _ = client.add_file("helper.mc").await;

    let content = std::fs::read_to_string(&path).expect("fixture exists");
    let sem = client.sem("helper.mc", Some(&content)).await.unwrap();

    // Build WorkspaceState with real RPC data
    let state = Arc::new(mcodels::WorkspaceState::new());
    let uri = tower_lsp::lsp_types::Url::parse("file:///helper.mc").unwrap();
    state.insert_document(uri.clone(), ropey::Rope::from_str(&content), 1);

    let rpc_tokens = mcodels::state::RpcSemTokens {
        tokens: sem
            .tokens
            .iter()
            .map(|t| mcodels::state::SemTokenEntry {
                type_: t.token_type,
                position: t.position,
                length: t.length,
            })
            .collect(),
    };
    state
        .symbols
        .sem_tokens
        .insert(uri.clone(), Arc::new(std::sync::Mutex::new(rpc_tokens)));

    let rpc_symbols = mcodels::state::RpcSemSymbols::from(sem.symbols);
    state
        .symbols
        .sem_symbols
        .insert(uri.clone(), Arc::new(std::sync::Mutex::new(rpc_symbols)));

    // Compute LSP semantic tokens from real mcc data
    let tokens = mcodels::features::semtok::compute(&state, &uri).unwrap();
    assert!(
        !tokens.is_empty(),
        "should produce LSP tokens for helper.mc"
    );
    // Every token should have a valid type
    for t in &tokens {
        assert!(t.length > 0, "token length must be > 0");
    }

    server.stop().await.ok();
}

#[tokio::test]
async fn completion_returns_keywords() {
    let mut server = mcodels::mccsrv::MccServer::new();
    server.start().await.expect("mcc server should start");
    assert!(wait_connected(&server).await);

    // Completion should work with keyword-only data (no mcc symbols needed)
    let state = Arc::new(mcodels::WorkspaceState::new());
    let uri = tower_lsp::lsp_types::Url::parse("file:///empty.mc").unwrap();
    state.insert_document(uri.clone(), ropey::Rope::from_str("\n"), 1);

    let params = tower_lsp::lsp_types::TextDocumentPositionParams {
        text_document: tower_lsp::lsp_types::TextDocumentIdentifier { uri: uri.clone() },
        position: tower_lsp::lsp_types::Position::new(0, 0),
    };
    let result = mcodels::features::comp::resolve(&state, &params);
    assert!(result.is_some(), "completion should return keyword items");

    server.stop().await.ok();
}
