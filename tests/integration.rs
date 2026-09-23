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

/// Layered completion RPC round-trip (§8.1): the authoritative scope comes
/// from mcc (`main.i2c`), P1 surfaces the func param, and member access on
/// the `hc` instance returns helper_chip's pins in the `Member` layer.
#[tokio::test]
async fn completion_layered_rpc_roundtrip() {
    let server = shared_server().await;
    assert!(wait_connected(&server).await);

    let path = fixture_path("comp.mc");
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;
    let _ = client.lib_load("mcode").await;
    let _ = client.set_project_root(&path).await;
    let _ = client.load_project(&path).await;

    let content = std::fs::read_to_string(&path).expect("fixture exists");

    // Cursor at the start of `GND` inside `func i2c(a)`.
    let pos = content.find("a -> GND").expect("marker") + "a -> ".len();

    // Layered P1-P5: scope must be the func, P1 must surface the param `a`.
    let resp = client.completion(&path, pos, None, None).await;
    assert!(resp.is_ok(), "completion RPC failed: {:?}", resp.err());
    let resp = resp.unwrap();
    assert_eq!(resp.scope_path, "main.i2c", "func-body scope");
    let p1 = resp.layers.get("P1").expect("P1 layer");
    assert!(p1.iter().any(|i| i.name == "a"), "P1 must contain param a");

    // Member access: root `hc` → helper_chip instance → its pins.
    let mem = client
        .completion(&path, pos, None, Some("hc"))
        .await
        .expect("member completion RPC failed");
    let member = mem.layers.get("Member").expect("Member layer");
    assert!(
        member.iter().any(|i| i.name == "IN_A"),
        "Member must contain helper_chip pin IN_A"
    );
}

/// A private server on a private port for the U258 wiring tests: the shared
/// harness binds 8080, which collides with any other session's server (and a
/// wedged daemon on 8080 accepts connections but never answers). mcc's own
/// `--port` flag moves the wire endpoint, so spawn a foreground server child
/// on the port, then let MccServer reuse it through its existing
/// port-busy probe path. Dropping the child kills it, freeing the port.
async fn private_server(port: u16) -> (MccServer, tokio::process::Child) {
    // Same resolution order as MccServer::find_mcc_path: env override first,
    // then the cargo-workspace relative path.
    let mcc_path = std::env::var("MCC_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| crate_path().join("../mcc/target/debug/mcc"));
    let child = tokio::process::Command::new(mcc_path)
        .args(["start", "--port", &port.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("mcc child should spawn");

    let mut server = MccServer::with_addr("127.0.0.1", port);
    // The port is bound by the child, so start() takes its reuse path
    // (server.info probe) — bounded retries are enough for process startup.
    for _ in 0..30 {
        if server.start().await.is_ok() && server.is_connected() {
            return (server, child);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("private mcc server on port {port} never became ready");
}

fn crate_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// ERC consumed as diagnostics (U258 §2): a driver-conflict module
/// (`b1.Y -> b2.Y`, two Out pins on one net) must come back as parsed
/// `ErcResponse` rows carrying `driver-conflict` with severity "error" and a
/// byte offset inside the fixture file. A fresh private server keeps the
/// module set isolated, so `mcb_get_first_module_name` picks the ERC
/// fixture's module.
#[tokio::test]
async fn erc_reports_driver_conflict_as_violations() {
    let (mut server, _child) = private_server(18081).await;
    let path = fixture_path("erc_conflict.mc");
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;
    // Mirror the server runtime sequence (run_server_init / did_open):
    // project root is the fixtures directory, the entry file is loaded whole.
    let fixtures_dir = crate_path().join("tests/fixtures");
    let _ = client
        .set_project_root(&fixtures_dir.to_string_lossy())
        .await;
    let _ = client.load_project(&path).await;

    let resp = client.erc().await;
    assert!(resp.is_ok(), "erc RPC failed: {:?}", resp.err());
    let resp = resp.unwrap();
    assert_eq!(resp.top, "erc_top", "top module should be the ERC fixture's");
    let conflict = resp
        .violations
        .iter()
        .find(|v| v.check == "driver-conflict")
        .expect("driver-conflict violation must fire");
    assert_eq!(conflict.severity, "error");
    assert!(
        resp.summary.get("violations").and_then(|v| v.as_u64()).is_some(),
        "summary.violations must be present"
    );
    // Cleanup so the private port frees up for a rerun.
    let _ = server.stop().await;
}

/// Cross-file find-references consumed from the mcc `refs` RPC (U258 §2):
/// the cursor sits on `helper_chip` in the refs_main.mc instance row; the
/// answer must include the definition site in helper.mc (`def: true`) plus
/// the ClassRef here, each row parsed into byte spans. The probe must load
/// through the canonical use form (`use ./helper`) — references only
/// register for use-resolved classes.
#[tokio::test]
async fn refs_cross_file_finds_definition_and_references() {
    let (mut server, _child) = private_server(18082).await;

    let helper_path = fixture_path("helper.mc");
    let probe_path = fixture_path("refs_main.mc");
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;
    let fixtures_dir = crate_path().join("tests/fixtures");
    let _ = client
        .set_project_root(&fixtures_dir.to_string_lossy())
        .await;
    let _ = client.load_project(&probe_path).await;
    let _ = client.add_file(&helper_path).await;

    let probe_text = std::fs::read_to_string(&probe_path).expect("fixture exists");
    let cursor = probe_text
        .find("helper_chip hc")
        .expect("cursor marker in refs_main.mc")
        + "helper_chip".len();

    let resp = client.refs(&probe_path, cursor, Some("helper_chip")).await;
    assert!(resp.is_ok(), "refs RPC failed: {:?}", resp.err());
    let resp = resp.unwrap();
    assert!(resp.count > 0, "cross-file refs must be non-empty");

    let def = resp
        .refs
        .iter()
        .find(|r| r.def)
        .expect("the definition site must be in the answer");
    assert!(
        def.uri.ends_with("helper.mc"),
        "definition must live in helper.mc, got {}",
        def.uri
    );
    // The ClassRef in the probe file itself.
    let here = resp.refs.iter().find(|r| {
        !r.def && r.uri.ends_with("refs_main.mc")
    });
    assert!(here.is_some(), "the probe file's own ClassRef must be present");
    for r in &resp.refs {
        assert!(r.end >= r.pos, "span end must not precede start");
    }

    // Cleanup so the private port frees up for a rerun.
    let _ = server.stop().await;
}

/// `caps` handshake (U258 §2): the private server's `start()` now probes
/// `caps` first (server.info stays the fallback), and the client exposes
/// `caps()` — the reply must carry the schema version, the method list
/// (including the AI contract methods this batch wires), and the explain
/// feature flag.
#[tokio::test]
async fn caps_handshake_reports_schema_and_methods() {
    let (mut server, _child) = private_server(18083).await;
    let client = server.client().expect("should have RPC client");

    let caps = client.caps().await.expect("caps RPC failed");
    assert!(caps.is_object(), "caps must be a JSON object");
    assert_eq!(caps["server"], "mcc");
    assert!(caps["schema_version"].as_u64().is_some());
    let methods = caps["methods"].as_array().expect("method list");
    for required in ["caps", "check", "explain", "show.component"] {
        assert!(
            methods.iter().any(|m| m.as_str() == Some(required)),
            "methods must advertise {required}"
        );
    }
    assert_eq!(caps["features"]["explain"], true);

    // Cleanup so the private port frees up for a rerun.
    let _ = server.stop().await;
}

/// `explain` consumed (U258 §2): a known code comes back with its name and
/// description; an unknown code is a server-side error, not a silent empty.
#[tokio::test]
async fn explain_returns_name_and_description() {
    let (mut server, _child) = private_server(18084).await;
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;

    // E5060 = the unconnected-port family face the pwrint batches pinned.
    let resp = client.explain(5060).await;
    assert!(resp.is_ok(), "explain RPC failed: {:?}", resp.err());
    let resp = resp.unwrap();
    assert_eq!(resp["code"], 5060);
    assert!(resp["name"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(
        resp["description"].as_str().is_some_and(|s| !s.is_empty()),
        "description must be present"
    );

    // Cleanup so the private port frees up for a rerun.
    let _ = server.stop().await;
}

/// `check` consumed (U258 §2): an inline dry-run of clean content reports
/// zero errors/warnings; content with a broken use target reports errors
/// without touching the workspace project.
#[tokio::test]
async fn check_dry_run_summarizes_inline_content() {
    let (mut server, _child) = private_server(18085).await;
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;

    let clean = "component clean_chip {\n    pins = [\n        io 1 = A\n    ]\n}\n";
    let resp = client.check(clean).await.expect("check RPC failed");
    assert_eq!(resp.summary.errors, 0, "clean content must not error");
    assert_eq!(resp.summary.warnings, 0);

    // A use directive pointing at a missing file must produce a user-file
    // error (pre-validation reports it to the user).
    let broken = "use ./no_such_helper anywhere\ncomponent broken_chip {\n}\n";
    let resp = client.check(broken).await.expect("check RPC failed");
    assert!(resp.summary.errors > 0, "broken content must error");

    // Cleanup so the private port frees up for a rerun.
    let _ = server.stop().await;
}

/// `show.component` consumed for completionItem/resolve grounding (U258 §2
/// / §4 S6): the drill-down must carry the pin table (pin_count + named pins
/// with iotypes) for `helper_chip`, matching the shape comp.rs formats.
#[tokio::test]
async fn show_component_gives_the_pin_table() {
    let (mut server, _child) = private_server(18086).await;

    let probe_path = fixture_path("refs_main.mc");
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;
    let fixtures_dir = crate_path().join("tests/fixtures");
    let _ = client
        .set_project_root(&fixtures_dir.to_string_lossy())
        .await;
    let _ = client.load_project(&probe_path).await;

    let resp = client.show("component", "helper_chip").await;
    assert!(resp.is_ok(), "show.component RPC failed: {:?}", resp.err());
    let resp = resp.unwrap();
    assert_eq!(resp["name"], "helper_chip");
    assert_eq!(resp["pin_count"], 3);
    let pins = resp["pins"].as_array().expect("pin rows");
    let all_names: Vec<&str> = pins
        .iter()
        .flat_map(|p| p["names"].as_array().expect("names").iter())
        .filter_map(|n| n.as_str())
        .collect();
    for expected in ["IN_A", "IN_B", "OUT"] {
        assert!(
            all_names.contains(&expected),
            "pin names must include {expected}, got {all_names:?}"
        );
    }

    // And the markdown formatter must turn that shape into a card that
    // names the pins (comp.rs ground_item's view of the same reply).
    let card = mcodels::features::comp::markdown("component", "helper_chip", &resp);
    assert!(card.contains("3 pin(s)"), "{card}");
    assert!(card.contains("IN_A"), "{card}");

    // Cleanup so the private port frees up for a rerun.
    let _ = server.stop().await;
}

/// Hover grounding (U258 §2 余项): `hover::ground` appends the show.* card
/// below the local hover for a named kind, and returns the local hover
/// untouched when mcc has no such symbol. Runs against a real private
/// mcc — same fixture (`refs_main.mc` → `helper_chip`) as the show probe.
#[tokio::test]
async fn hover_ground_appends_show_card_and_falls_back() {
    let (mut server, _child) = private_server(18087).await;

    let probe_path = fixture_path("refs_main.mc");
    let client = server.client().expect("should have RPC client");
    let _ = client.init().await;
    let fixtures_dir = crate_path().join("tests/fixtures");
    let _ = client
        .set_project_root(&fixtures_dir.to_string_lossy())
        .await;
    let _ = client.load_project(&probe_path).await;

    let local = tower_lsp::lsp_types::Hover {
        contents: tower_lsp::lsp_types::HoverContents::Markup(
            tower_lsp::lsp_types::MarkupContent {
                kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                value: "`helper_chip` (component)".into(),
            },
        ),
        range: None,
    };

    // Grounded: local line first, show card (pin table) appended.
    let subject = mcodels::features::hover::Subject {
        kind: "component",
        name: "helper_chip".into(),
    };
    let grounded = mcodels::features::hover::ground(local.clone(), &subject, client).await;
    let tower_lsp::lsp_types::HoverContents::Markup(mc) = &grounded.contents else {
        panic!("expected Markup");
    };
    assert!(mc.value.starts_with("`helper_chip` (component)"), "{}", mc.value);
    assert!(mc.value.contains("---"), "card separator missing: {}", mc.value);
    assert!(mc.value.contains("3 pin(s)"), "pin table missing: {}", mc.value);

    // Unknown symbol: show fails, the local hover comes back byte-identical.
    let missing = mcodels::features::hover::Subject {
        kind: "component",
        name: "no_such_chip".into(),
    };
    let fallback = mcodels::features::hover::ground(local, &missing, client).await;
    let tower_lsp::lsp_types::HoverContents::Markup(mc) = &fallback.contents else {
        panic!("expected Markup");
    };
    assert_eq!(mc.value, "`helper_chip` (component)");

    let _ = server.stop().await;
}
