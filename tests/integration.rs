//! Integration tests with a real mcc server subprocess.
//!
//! Prerequisites: `mcc` binary at `../mcc/target/debug/mcc` or on PATH.
//! These tests verify the RPC pipeline: start server → load fixture → call features.
//!
//! The mcc server binds to a fixed port (8080) inside the `mcc` binary, so we
//! cannot run multiple `MccServer` instances in parallel. All tests in this
//! file therefore share a single `MccServer` via a process-level `OnceLock`.
//!
//! The `mcc` binary also writes a PID file at `~/.mcode/logs/mcc.pid` and
//! refuses to start if it sees a stale entry from a previous run that did
//! not clean up (e.g. a SIGKILL on the test process). The `cleanup_stale_mcc_pid_file`
//! helper below removes such entries before the shared server is started.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::{Mutex, MutexGuard};

use mcodels::mccsrv::MccServer;

/// Path to the `mcc` PID file. Mirrors `pid_file_path()` in `mcc/src/cli/datadir.rs`.
fn mcc_pid_file() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home)
        .join(".mcode")
        .join("logs")
        .join("mcc.pid")
}

/// Remove the stale `mcc.pid` file left behind by a previous mcc run that
/// did not shut down cleanly. mcc refuses to start when it sees a stale
/// PID, and macOS zombies can keep a PID "alive" for `kill -0` purposes
/// even after the process has been reaped.
///
/// Safe to call when no `mcc` server is running: the file simply won't exist
/// (or will be removed and the absence is harmless). Refuses to remove the
/// file if a real, non-zombie mcc process is alive (so we never clobber a
/// running server owned by the user).
fn cleanup_stale_mcc_pid_file() {
    let path = mcc_pid_file();
    if !path.exists() {
        return;
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let pid: u32 = match content.lines().next().and_then(|l| l.trim().parse().ok()) {
        Some(p) => p,
        None => {
            // Malformed PID file — safe to remove.
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    // Use `ps -o stat=` to detect zombies (state "Z"). `kill -0` returns
    // success for zombies, so it cannot be used alone to determine liveness.
    let ps_output = std::process::Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("stat=")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    let alive = match ps_output {
        Ok(out) if out.status.success() => {
            let state = String::from_utf8_lossy(&out.stdout);
            let state = state.trim();
            !state.is_empty() && state != "Z" && state != "U"
        }
        // ps returns non-zero when the PID doesn't exist at all.
        _ => false,
    };
    if !alive {
        let _ = std::fs::remove_file(&path);
    }
}

/// Process-wide shared mcc server. Initialized lazily on first access.
static SERVER: OnceLock<Mutex<MccServer>> = OnceLock::new();

fn shared_server_slot() -> &'static Mutex<MccServer> {
    SERVER.get_or_init(|| Mutex::new(MccServer::new()))
}

/// Acquire the shared server, starting it on first use. The guard is held
/// for the duration of the test so concurrent tests serialize on the server
/// mutex; the underlying mcc subprocess is started once and reused.
async fn shared_server() -> MutexGuard<'static, MccServer> {
    let guard = shared_server_slot().lock().await;
    // We must NOT hold the lock while calling `start()` (which can block on
    // subprocess I/O), so drop it and re-acquire after starting.
    drop(guard);

    let mut guard = shared_server_slot().lock().await;
    if !guard.is_connected() {
        cleanup_stale_mcc_pid_file();
        guard
            .start()
            .await
            .expect("mcc server should start (shared)");
    }
    guard
}

async fn wait_connected(server: &MccServer) -> bool {
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
    let server = shared_server().await;
    assert!(
        wait_connected(&server).await,
        "server should connect within 10s"
    );
    // Note: do NOT stop the server — other tests in this file reuse it.
}

#[tokio::test]
async fn sem_returns_tokens_and_symbols() {
    let server = shared_server().await;
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
        sem.symbols.lapper.iter().any(|e| e.kind == 0),
        "helper_chip should produce a class_def lapper entry (kind=0)"
    );
}

#[tokio::test]
async fn diagnostics_no_error_for_valid_file() {
    let server = shared_server().await;
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
}

#[tokio::test]
async fn sem_tokens_compute_from_real_data() {
    let server = shared_server().await;
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
}

#[tokio::test]
async fn completion_returns_keywords() {
    // This test does not use the mcc server (comp uses keyword-only data).
    // We still take the server lock briefly to make sure the server is up,
    // so all tests in this file observe a consistent "server ready" state.
    let _server = shared_server().await;

    let state = Arc::new(mcodels::WorkspaceState::new());
    let uri = tower_lsp::lsp_types::Url::parse("file:///empty.mc").unwrap();
    state.insert_document(uri.clone(), ropey::Rope::from_str("\n"), 1);

    let params = tower_lsp::lsp_types::TextDocumentPositionParams {
        text_document: tower_lsp::lsp_types::TextDocumentIdentifier { uri: uri.clone() },
        position: tower_lsp::lsp_types::Position::new(0, 0),
    };
    let result = mcodels::features::comp::resolve(&state, &params);
    assert!(result.is_some(), "completion should return keyword items");
}
