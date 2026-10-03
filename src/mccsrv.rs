//! MCC server manager with subprocess and RPC
//!
//! Manages an `mcc server` subprocess:
//! - Discovers the slot's daemon through its pid file
//! - Spawns mcc server process when the slot has none
//! - Provides RPC client for communication
//! - Auto-restarts on crash
//!
//! This solves two problems:
//! 1. Logs visible in this process's output (vs embedded in LSP)
//! 2. Crash isolation - mcc crash only kills subprocess, not LSP

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

use crate::rpc::MccRpcClient;

/// The wire protocol version this client speaks (mcc `buildinfo::RPC_PROTOCOL`).
const RPC_PROTOCOL: &str = "mcc-rpc/1";

/// MCC server connection state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Crashed,
}

/// One daemon slot: the discovery record plus the spawn context.
///
/// Slot law (mcc `world-sandbox-design.md` §4.1, ruling 1): the slot
/// key is the project root, and the slot's pid file is the *only* discovery
/// record — port probing cannot tell *which* project's daemon answered, so
/// it is used only as the default-root well-known fallback.
struct Slot {
    pid_path: PathBuf,
    /// Spawn cwd. `Some` only for a project slot (the directory holding
    /// `project.toml`) — the daemon derives its own slot from its cwd, so
    /// the spawned process and the discovery record must agree.
    cwd: Option<PathBuf>,
    /// Well-known address tried when the pid file does not exist at all
    /// (default-root slot only; a daemon there binds the historical :8080).
    fallback: Option<(String, u16)>,
}

impl Slot {
    /// Derive the slot for a workspace root. `None` (no project manifest in
    /// reach) lands on the default-root slot: pid file at
    /// `$MCC_SYSTEM_ROOT/logs/mcc.pid` (or the global `~/.mcode/logs/mcc.pid`)
    /// plus the historical :8080 well-known address. With no `HOME` either,
    /// the pid path degrades to a temp dir — discovery then simply misses and
    /// the fallback address carries the connection, matching mcc's own
    /// datadir fallback shape.
    fn for_root(project_root: Option<&Path>) -> Slot {
        match project_root {
            Some(root) => Slot {
                pid_path: root.join(".mcode").join("mcc.pid"),
                cwd: Some(root.to_path_buf()),
                fallback: None,
            },
            None => {
                let base = std::env::var_os("MCC_SYSTEM_ROOT")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".mcode")))
                    .unwrap_or_else(std::env::temp_dir);
                Slot {
                    pid_path: base.join("logs").join("mcc.pid"),
                    cwd: None,
                    fallback: Some((MccServer::DEFAULT_HOST.to_string(), MccServer::DEFAULT_PORT)),
                }
            }
        }
    }
}

/// One record line pair from a slot pid file (`pid` + `host:port`, the
/// format mcc's `write_pid_file` lays down).
struct PidRecord {
    pid: u32,
    host: String,
    port: u16,
}

/// Read and parse a slot pid file. `None` when absent or malformed — both
/// mean "no usable discovery record".
fn read_pid_record(path: &Path) -> Option<PidRecord> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let addr = lines.next()?.trim();
    let (host, port) = addr.split_once(':')?;
    Some(PidRecord {
        pid,
        host: host.to_string(),
        port: port.parse().ok()?,
    })
}

/// Liveness probe via `kill -0` (same technique as mcc's own stop/status
/// faces): true when the pid exists and we may signal it.
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Stop a daemon we discovered through the slot's pid file (§4.4 restart
/// flow): SIGTERM, wait up to 5 s, escalate to SIGKILL, then drop the stale
/// pid record. Only ever called for a daemon this slot's own pid file
/// vouched for.
fn stop_daemon(rec: &PidRecord, pid_path: &Path) {
    let kill = |signal: &str| {
        let _ = std::process::Command::new("kill")
            .arg(signal)
            .arg(rec.pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();
    };
    kill("-TERM");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !process_alive(rec.pid) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if process_alive(rec.pid) {
        warn!("daemon pid {} survived SIGTERM, sending SIGKILL", rec.pid);
        kill("-KILL");
    }
    let _ = std::fs::remove_file(pid_path);
}

/// The client handshake triple: what this front-end would spawn, parsed from
/// `mcc --version` (`mcc 0.9.1.b4546`). Comparing the daemon against the
/// local binary is the point — drift means the daemon is not the build this
/// front-end knows, and the restart flow converges the slot onto the local
/// binary.
struct ClientTriple {
    mcc_version: String,
    build: u64,
}

impl ClientTriple {
    fn probe() -> Option<ClientTriple> {
        let out = std::process::Command::new(MccServer::find_mcc_path())
            .arg("--version")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let (mcc_version, build) = parse_version_output(&String::from_utf8_lossy(&out.stdout))?;
        Some(ClientTriple { mcc_version, build })
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "protocol": RPC_PROTOCOL,
            "mcc_version": self.mcc_version,
            "build": self.build,
        })
    }
}

/// Parse `mcc 0.9.1.b4546` into `(version, build)`.
fn parse_version_output(out: &str) -> Option<(String, u64)> {
    let token = out.split_whitespace().find(|t| t.contains('.'))?;
    let mut parts = token.splitn(4, '.');
    let (maj, min, patch, build) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    if maj.is_empty() || min.is_empty() || patch.is_empty() {
        return None;
    }
    Some((
        format!("{maj}.{min}.{patch}"),
        build.strip_prefix('b')?.parse().ok()?,
    ))
}

/// Outcome of one handshake attempt against a live daemon.
enum HandshakeVerdict {
    /// Triple matched, verdict missing (daemon predates the handshake), or
    /// `unverified` (client could not identify itself) — reuse the daemon.
    Accept,
    /// Version/build drift (`version_mismatch`/`stale_build`): revision
    /// tokens are build-scoped, so the daemon must be restarted before this
    /// client can trust its reads. Per §4.4 (ruling 3) the front-end owns
    /// reconnection: stop the slot daemon through its pid record, spawn a
    /// fresh one from the local binary, re-handshake. The slot daemon serves
    /// this project — restarting it is the sanctioned flow, and a peer
    /// front-end recovers through its own cold-reconnect path.
    Refuse(String),
    /// `protocol_mismatch`: the wire protocol itself differs — restarting
    /// cannot help (the local binary speaks the same protocol this client
    /// does, so a restart would converge, but a protocol break means the
    /// front-end itself is out of date). Surface the refusal; the caller
    /// falls back to direct mode.
    Fatal(String),
}

/// Read the §4.3 verdict out of a `caps` reply. `fallback` marks the
/// `server.info` path (daemon predates the handshake) — legacy ground,
/// always accept.
fn verdict_from_reply(reply: &serde_json::Value, fallback: bool) -> HandshakeVerdict {
    if fallback {
        return HandshakeVerdict::Accept;
    }
    match reply
        .get("handshake")
        .and_then(|h| h.get("verdict"))
        .and_then(|v| v.as_str())
    {
        Some("protocol_mismatch") => HandshakeVerdict::Fatal(
            reply
                .get("handshake")
                .and_then(|h| h.get("detail"))
                .and_then(|d| d.as_str())
                .unwrap_or("protocol mismatch")
                .to_string(),
        ),
        Some(v @ ("stale_build" | "version_mismatch")) => HandshakeVerdict::Refuse(format!(
            "{}: {}",
            v,
            reply
                .get("handshake")
                .and_then(|h| h.get("detail"))
                .and_then(|d| d.as_str())
                .unwrap_or("build drift")
        )),
        _ => HandshakeVerdict::Accept, // ok / unverified / legacy sheet
    }
}

/// MCC server manager
pub struct MccServer {
    /// RPC client
    client: Option<MccRpcClient>,
    /// mcc subprocess handle (kept alive to prevent kill_on_drop)
    child: Option<Child>,
    /// Server host:port
    host: String,
    port: u16,
    /// Connection state
    state: ConnectionState,
    /// Number of restart attempts
    restart_count: u32,
    /// Max restart attempts before giving up
    max_restarts: u32,
    /// `true` when the address was pinned by the caller ([`Self::with_addr`]):
    /// connects to exactly that address and never restarts a daemon it finds
    /// there (integration tests pin private daemons this way).
    explicit_addr: bool,
    /// The workspace root this connection was started for; the cold
    /// reconnect re-runs discovery against the same slot.
    slot_root: Option<PathBuf>,
    /// System root stored at init time so the cold reconnect can replay the
    /// same `set_system_root` without a snapshot from the caller.
    system_root: Option<PathBuf>,
    /// Set when an RPC fails at the connection level (daemon died). The
    /// client then reads as disconnected and the publish path schedules one
    /// cold reconnect — crash recovery is a fresh start, since the world is
    /// a deterministic function of the on-disk sources (no shared state to
    /// salvage).
    dead: AtomicBool,
    /// Exactly-once latch for the cold reconnect: the first caller to win
    /// the [`Self::schedule_reconnect`] race spawns the reconnect task;
    /// losers keep failing fast until the latch clears.
    reconnecting: AtomicBool,
}

impl MccServer {
    /// Default MCC server host
    pub const DEFAULT_HOST: &'static str = "127.0.0.1";
    /// Default MCC server port (the default-root well-known port)
    pub const DEFAULT_PORT: u16 = 8080;
    /// Startup timeout (reserved for future use)
    #[allow(dead_code)]
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

    /// Create new server manager
    pub fn new() -> Self {
        Self {
            client: None,
            child: None,
            host: Self::DEFAULT_HOST.to_string(),
            port: Self::DEFAULT_PORT,
            state: ConnectionState::Disconnected,
            restart_count: 0,
            max_restarts: 3,
            explicit_addr: false,
            slot_root: None,
            system_root: None,
            dead: AtomicBool::new(false),
            reconnecting: AtomicBool::new(false),
        }
    }

    /// Create with custom host/port
    pub fn with_addr(host: &str, port: u16) -> Self {
        let mut s = Self::new();
        s.host = host.to_string();
        s.port = port;
        s.explicit_addr = true;
        s
    }

    /// Clear the log file at start (useful when LSP restarts and reconnects to existing mcc)
    pub fn clear_log() {
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            let log_path = std::path::Path::new(&manifest).join("log.txt");
            if let Err(e) = std::fs::write(&log_path, "") {
                warn!("failed to clear log file: {e}");
            }
        }
    }

    /// Handshake probe for connection attempts. With a [`ClientTriple`] this
    /// is the formal §4.3 handshake (`caps` + `client` → verdict); the plain
    /// probe keeps `server.info` as the fallback so an older mcc binary that
    /// predates `caps` still connects. Returns the reply plus whether the
    /// legacy fallback path served it.
    async fn handshake(
        client: &MccRpcClient,
        triple: Option<&ClientTriple>,
    ) -> Result<(serde_json::Value, bool), crate::rpc::RpcError> {
        match timeout(
            Duration::from_secs(2),
            client.caps(triple.map(ClientTriple::to_json)),
        )
        .await
        {
            Ok(Ok(caps)) => Ok((caps, false)),
            Ok(Err(e)) => {
                debug!("caps handshake failed ({e}); falling back to server.info");
                match timeout(
                    Duration::from_secs(2),
                    client.call("server.info", serde_json::json!({})),
                )
                .await
                {
                    Ok(Ok(info)) => Ok((info, true)),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(crate::rpc::RpcError::Network(
                        "handshake timeout (caps + server.info)".to_string(),
                    )),
                }
            }
            Err(_) => Err(crate::rpc::RpcError::Network(
                "caps handshake timeout".to_string(),
            )),
        }
    }

    /// Start mcc server subprocess and connect. `project_root` selects the
    /// slot (§4.1): `Some` = project slot (pid file `<root>/.mcode/mcc.pid`,
    /// spawn cwd = the project, daemon binds port 0), `None` = default-root
    /// slot (global pid file + :8080 well-known). A pinned address
    /// ([`Self::with_addr`]) bypasses slot discovery entirely.
    pub async fn start(&mut self, project_root: Option<&Path>) -> Result<(), MccServerError> {
        // Clear log at start of each session
        Self::clear_log();

        // A fresh start attempt wipes a stale connection-loss mark; the
        // early return below must not fire while `dead` is set, or the cold
        // reconnect would no-op against a corpse.
        self.clear_dead();

        if self.state == ConnectionState::Connected {
            return Ok(());
        }

        warn!("=== MccServer::start called ===");

        if self.explicit_addr {
            return self.start_pinned().await;
        }

        self.slot_root = project_root.map(Path::to_path_buf);
        self.start_slot(&Slot::for_root(project_root)).await
    }

    /// Pinned-address path (integration tests): legacy semantics — reuse
    /// whatever answers on the pinned address, spawn there when the port is
    /// free, never restart a daemon we find. The handshake is logged but not
    /// enforced: the pin is the caller's contract.
    async fn start_pinned(&mut self) -> Result<(), MccServerError> {
        let check_addr = format!("{}:{}", self.host, self.port);
        if std::net::TcpListener::bind(&check_addr).is_err() {
            info!("Port {} is already in use, trying to connect", self.port);
            match MccRpcClient::new(&self.host, self.port) {
                Ok(client) => {
                    match timeout(Duration::from_secs(2), Self::handshake(&client, None)).await {
                        Ok(Ok((caps, _))) => {
                            info!("Connected to existing mcc server (caps: {})", caps);
                            self.client = Some(client);
                            self.state = ConnectionState::Connected;
                            return Ok(());
                        }
                        Ok(Err(e)) => {
                            warn!("mcc server responded with error: {}", e);
                        }
                        Err(_) => {
                            warn!("Timeout connecting to existing mcc server");
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to create RPC client for existing server: {}", e);
                }
            }
        } else {
            info!("Port {} is free, will spawn new mcc server", self.port);
        }
        self.spawn_loop(None, None).await
    }

    /// Slot path (§4.1 discovery law): pid file is the only record, port
    /// probing never *adopts* a daemon — at most the default-root fallback
    /// address answers an orphan whose pid record is gone.
    async fn start_slot(&mut self, slot: &Slot) -> Result<(), MccServerError> {
        // The triple anchor is the local binary we would spawn. Without it
        // we cannot judge drift — skip reuse and let the spawn path converge
        // the slot onto whatever binary is deployed.
        let triple = ClientTriple::probe();
        if triple.is_none() {
            warn!(
                "cannot determine local mcc version ({} --version failed); skipping daemon reuse",
                Self::find_mcc_path().display()
            );
        }

        // 1. Discovery record → live daemon → handshake.
        if let Some(rec) = read_pid_record(&slot.pid_path) {
            if process_alive(rec.pid) {
                match self.reuse_discovered(rec, slot, triple.as_ref()).await {
                    Ok(()) => return Ok(()),
                    Err(ReuseError::Fatal(e)) => return Err(e),
                    Err(ReuseError::Restarted) => {} // §4.4 restart done, fall through to spawn
                }
            } else {
                info!(
                    "stale pid record (pid {} gone), removing {}",
                    rec.pid,
                    slot.pid_path.display()
                );
                let _ = std::fs::remove_file(&slot.pid_path);
            }
        }

        // 2. Default-root edge: no live pid record, but the well-known port
        //    answers — an orphan daemon whose pid record was lost. Judge it
        //    by handshake but never kill a process our pid file did not
        //    vouch for.
        if let Some((ref host, port)) = slot.fallback {
            if read_pid_record(&slot.pid_path).is_none()
                && std::net::TcpListener::bind((host.as_str(), port)).is_err()
            {
                warn!(
                    "no pid record but {}:{} answers; judging orphan daemon by handshake",
                    host, port
                );
                match self.reuse_orphan(host, port, triple.as_ref()).await {
                    Ok(()) => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
        }

        // 3. Empty (or just-restarted) slot: spawn fresh.
        self.spawn_loop(Some(slot), triple.as_ref()).await
    }

    /// Handshake a daemon found through the slot pid file. Accept reuses it;
    /// Refuse runs the §4.4 restart (stop via pid record, then fall through
    /// to spawn); Fatal propagates.
    async fn reuse_discovered(
        &mut self,
        rec: PidRecord,
        slot: &Slot,
        triple: Option<&ClientTriple>,
    ) -> Result<(), ReuseError> {
        info!(
            "slot daemon discovered via {}: pid {} at {}:{}",
            slot.pid_path.display(),
            rec.pid,
            rec.host,
            rec.port
        );
        let client = MccRpcClient::new(&rec.host, rec.port)
            .map_err(|e| ReuseError::Fatal(MccServerError::Rpc(e.to_string())))?;
        let (reply, fallback) =
            Self::handshake(&client, triple)
                .await
                .map_err(|e| ReuseError::Fatal(MccServerError::Rpc(e.to_string())))?;
        match verdict_from_reply(&reply, fallback) {
            HandshakeVerdict::Accept => {
                info!(
                    "handshake accepted, reusing slot daemon at {}:{}",
                    rec.host, rec.port
                );
                self.client = Some(client);
                self.host = rec.host;
                self.port = rec.port;
                self.state = ConnectionState::Connected;
                Ok(())
            }
            HandshakeVerdict::Refuse(detail) => {
                warn!("handshake refused slot daemon, restarting it: {detail}");
                stop_daemon(&rec, &slot.pid_path);
                Err(ReuseError::Restarted)
            }
            HandshakeVerdict::Fatal(detail) => {
                error!("handshake fatal against slot daemon: {detail}");
                Err(ReuseError::Fatal(MccServerError::FailedToStart(format!(
                    "protocol mismatch with slot daemon: {detail}"
                ))))
            }
        }
    }

    /// Handshake an orphan on the fallback address (default root, no pid
    /// record). Accept reuses; drift returns an error pointing at
    /// `mcc restart` — we do not kill a process our pid file never vouched
    /// for.
    async fn reuse_orphan(
        &mut self,
        host: &str,
        port: u16,
        triple: Option<&ClientTriple>,
    ) -> Result<(), MccServerError> {
        let client = match MccRpcClient::new(host, port) {
            Ok(c) => c,
            Err(e) => return Err(MccServerError::Rpc(e.to_string())),
        };
        let (reply, fallback) = match Self::handshake(&client, triple).await {
            Ok(r) => r,
            Err(e) => return Err(MccServerError::Rpc(e.to_string())),
        };
        match verdict_from_reply(&reply, fallback) {
            HandshakeVerdict::Accept => {
                info!("handshake accepted, reusing orphan daemon at {host}:{port}");
                self.client = Some(client);
                self.host = host.to_string();
                self.port = port;
                self.state = ConnectionState::Connected;
                Ok(())
            }
            HandshakeVerdict::Refuse(detail) | HandshakeVerdict::Fatal(detail) => {
                error!(
                    "orphan daemon at {host}:{port} refused (no pid record to restart it): {detail}; run `mcc restart`"
                );
                Err(MccServerError::FailedToStart(format!(
                    "stale orphan daemon on the well-known port (run `mcc restart`): {detail}"
                )))
            }
        }
    }

    /// Spawn a fresh `mcc start` in the slot and connect. The readiness wait
    /// polls the slot pid file — the daemon's own record of the address it
    /// bound (the project slot asks for port 0, so the pid file is the only
    /// place the real port appears). `triple` enforces the §4.3 verdict on
    /// the daemon we just spawned: same binary, so drift here is an error,
    /// not a restart loop.
    async fn spawn_loop(
        &mut self,
        slot: Option<&Slot>,
        triple: Option<&ClientTriple>,
    ) -> Result<(), MccServerError> {
        // Use a loop for retries
        loop {
            self.state = ConnectionState::Connecting;
            info!(
                "Starting mcc server (attempt {}/{})...",
                self.restart_count + 1,
                self.max_restarts
            );

            // Find mcc binary. Absolute before spawn: the project slot sets
            // current_dir, and a relative program path would then resolve
            // against the *child's* cwd and miss.
            let mcc_path = Self::find_mcc_path();
            let mcc_path = mcc_path.canonicalize().unwrap_or(mcc_path);

            // Start server (correct command is "mcc start"); the project slot
            // spawns with cwd = project root so the daemon's own slot
            // derivation agrees with the discovery record (§4.1).
            info!("Spawning mcc from: {:?}", mcc_path);
            let mut cmd = Command::new(&mcc_path);
            cmd.arg("start")
                .stdout(Stdio::null()) // Don't capture stdout, let mcc write directly
                .stderr(Stdio::inherit()) // Inherit stderr for debugging
                .kill_on_drop(true);
            if let Some(dir) = slot.and_then(|s| s.cwd.as_ref()) {
                cmd.current_dir(dir);
            } else {
                // Pinned-address spawn: the child must answer on the pinned
                // address, not the default :8080.
                cmd.arg("--host").arg(&self.host).arg("--port").arg(self.port.to_string());
            }
            let mut child = match cmd.spawn() {
                Ok(c) => {
                    info!("mcc spawned successfully, pid={:?}", c.id());
                    c
                }
                Err(e) => {
                    error!("Failed to spawn mcc: {}", e);
                    self.restart_count += 1;
                    if self.restart_count >= self.max_restarts {
                        return Err(MccServerError::Spawn(e.to_string()));
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            // Wait for the daemon to settle: it either writes its pid record
            // (its bound address — authoritative when the project slot binds
            // port 0) or dies immediately.
            info!("Waiting for mcc to write its pid record...");
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut addr: Option<(String, u16)> = None;
            loop {
                if Instant::now() >= deadline {
                    break;
                }
                match child.try_wait() {
                    Ok(Some(status)) => {
                        error!("mcc process exited immediately: {:?}", status);
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!("Failed to check mcc status: {}", e);
                        break;
                    }
                }
                if let Some(s) = slot {
                    if let Some(rec) = read_pid_record(&s.pid_path) {
                        info!("pid record appeared: {}:{}", rec.host, rec.port);
                        addr = Some((rec.host, rec.port));
                        break;
                    }
                } else {
                    // Pinned address without a slot: the daemon binds the
                    // pinned port, so a connect probe is the readiness test.
                    addr = Some((self.host.clone(), self.port));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }

            let Some((host, port)) = addr else {
                self.restart_count += 1;
                if self.restart_count >= self.max_restarts {
                    return Err(MccServerError::FailedToStart(
                        "mcc died before writing its pid record".to_string(),
                    ));
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            };

            // Extra wait for mcc to fully initialize
            info!("Extra wait for mcc initialization...");
            tokio::time::sleep(Duration::from_secs(1)).await;

            // Try to connect with retries
            info!("Attempting RPC connection to {}:{}", host, port);
            let client = match MccRpcClient::new(&host, port) {
                Ok(c) => c,
                Err(e) => {
                    error!("Failed to build RPC client: {}", e);
                    self.restart_count += 1;
                    if self.restart_count >= self.max_restarts {
                        return Err(MccServerError::Rpc(e.to_string()));
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            for attempt in 1..=5 {
                info!("RPC connection attempt {}/5", attempt);
                match timeout(Duration::from_secs(2), Self::handshake(&client, triple)).await {
                    Ok(Ok((caps, fallback))) => match verdict_from_reply(&caps, fallback) {
                        HandshakeVerdict::Accept => {
                            self.client = Some(client);
                            self.child = Some(child);
                            self.host = host;
                            self.port = port;
                            self.state = ConnectionState::Connected;
                            self.restart_count = 0;
                            info!("mcc server connected at {}:{}", self.host, self.port);
                            info!("handshake: {}", caps);
                            return Ok(());
                        }
                        HandshakeVerdict::Refuse(detail) | HandshakeVerdict::Fatal(detail) => {
                            // The daemon was spawned from the same binary the
                            // triple was probed from — drift here means the
                            // probe lied or the slot is haunted; retrying
                            // cannot fix it.
                            error!("freshly spawned daemon refused handshake: {detail}");
                            return Err(MccServerError::FailedToStart(format!(
                                "spawned daemon drifted from the local binary: {detail}"
                            )));
                        }
                    },
                    Ok(Err(e)) => {
                        warn!("RPC attempt {} failed: {}", attempt, e);
                    }
                    Err(_) => {
                        warn!("RPC attempt {} timeout", attempt);
                    }
                }
                if attempt < 5 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }

            error!("Failed to connect to mcc server after 5 attempts");

            // Mark as crashed and retry
            self.state = ConnectionState::Crashed;
            self.restart_count += 1;

            if self.restart_count >= self.max_restarts {
                return Err(MccServerError::FailedToStart(
                    "max restart attempts exceeded".to_string(),
                ));
            }

            warn!("mcc server failed, retrying in 1s");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// Stop mcc server and kill the child process
    pub async fn stop(&mut self) -> Result<(), MccServerError> {
        if let Some(ref mut child) = self.child {
            if let Err(e) = child.start_kill() {
                warn!("failed to kill mcc child process: {}", e);
            }
            self.child = None;
        }
        self.state = ConnectionState::Disconnected;
        info!("mcc server stopped");
        Ok(())
    }

    /// Get RPC client. `None` while disconnected *or* marked dead — a
    /// transport failure flips both, so every RPC face fails fast instead of
    /// queuing behind a corpse connection.
    pub fn client(&self) -> Option<&MccRpcClient> {
        if self.state == ConnectionState::Connected && !self.connection_lost() {
            self.client.as_ref()
        } else {
            None
        }
    }

    /// Check if connected (a connection-loss mark reads as disconnected
    /// until the cold reconnect revives the slot).
    pub fn is_connected(&self) -> bool {
        self.state == ConnectionState::Connected && !self.connection_lost()
    }

    /// Get connection state
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// System root stored at init time; the cold reconnect replays it
    /// without a snapshot from the caller.
    pub fn set_system_root(&mut self, root: Option<PathBuf>) {
        self.system_root = root;
    }

    /// The workspace root this connection was started for (the cold
    /// reconnect re-runs discovery against the same slot).
    pub fn slot_root(&self) -> Option<&Path> {
        self.slot_root.as_deref()
    }

    /// The system root stored at init time.
    pub fn system_root(&self) -> Option<&Path> {
        self.system_root.as_deref()
    }

    /// Reset the restart budget before a fresh (re)start sequence.
    pub fn reset_restarts(&mut self) {
        self.restart_count = 0;
    }

    /// Record a transport-level connection loss (daemon unreachable).
    /// Idempotent; logs once. Every RPC face reads as disconnected until a
    /// cold reconnect succeeds.
    fn mark_dead(&self) {
        if !self.dead.swap(true, Ordering::Relaxed) {
            warn!("mcc daemon connection lost; failing reads fast until cold reconnect");
        }
    }

    /// Whether a transport-level connection loss has been recorded.
    pub fn connection_lost(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }

    /// Clear the connection-loss mark (a successful reconnect/start does
    /// this): RPC faces read as connected again.
    pub fn clear_dead(&self) {
        self.dead.store(false, Ordering::Relaxed);
    }

    /// Latch exactly one cold reconnect. `true` for the caller that won the
    /// race — it spawns the reconnect task; losers do nothing (the winner
    /// revives the shared connection).
    pub fn schedule_reconnect(&self) -> bool {
        self.reconnecting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Release the reconnect latch (reconnect finished or gave up): the
    /// next connection loss may schedule again.
    pub fn clear_reconnect_latch(&self) {
        self.reconnecting.store(false, Ordering::Relaxed);
    }

    /// Call sem RPC to get semantic data for a file
    pub async fn sem(
        &self,
        uri: &str,
        content: Option<&str>,
    ) -> Result<crate::rpc::SemResponse, MccServerError> {
        let client = self.client().ok_or(MccServerError::NotConnected)?;
        client
            .sem(uri, content)
            .await
            .map_err(|e| {
                if e.is_connection_loss() {
                    self.mark_dead();
                }
                MccServerError::Rpc(e.to_string())
            })
    }

    /// Call diagnostics RPC to get diagnostics for a file
    pub async fn diagnostics(
        &self,
        uri: &str,
    ) -> Result<crate::rpc::DiagnosticsResponse, MccServerError> {
        let client = self.client().ok_or(MccServerError::NotConnected)?;
        client
            .diagnostics(uri)
            .await
            .map_err(|e| {
                if e.is_connection_loss() {
                    self.mark_dead();
                }
                MccServerError::Rpc(e.to_string())
            })
    }

    /// Call `refs` RPC for position-aware cross-file find-references.
    pub async fn refs(
        &self,
        uri: &str,
        position: usize,
        name: Option<&str>,
    ) -> Result<crate::rpc::RefsResponse, MccServerError> {
        let client = self.client().ok_or(MccServerError::NotConnected)?;
        client
            .refs(uri, position, name)
            .await
            .map_err(|e| {
                if e.is_connection_loss() {
                    self.mark_dead();
                }
                MccServerError::Rpc(e.to_string())
            })
    }

    /// Call `erc` RPC to run the flat electrical net checks for the workspace.
    pub async fn erc(&self) -> Result<crate::rpc::ErcResponse, MccServerError> {
        let client = self.client().ok_or(MccServerError::NotConnected)?;
        client
            .erc()
            .await
            .map_err(|e| {
                if e.is_connection_loss() {
                    self.mark_dead();
                }
                MccServerError::Rpc(e.to_string())
            })
    }

    /// Call `build.viz` RPC to render a circuit to a self-contained HTML string.
    pub async fn build_viz(
        &self,
        entry: &str,
        top: Option<&str>,
        libs: &[String],
        layouter: Option<&str>,
    ) -> Result<String, MccServerError> {
        let client = self.client().ok_or(MccServerError::NotConnected)?;
        client
            .build_viz(entry, top, libs, layouter)
            .await
            .map_err(|e| {
                if e.is_connection_loss() {
                    self.mark_dead();
                }
                MccServerError::Rpc(e.to_string())
            })
    }

    /// Call `build.full` RPC to build the whole project (equivalent of
    /// `mcc build`), returning the structured per-phase envelope.
    pub async fn build_full(
        &self,
        entry: &str,
        top: Option<&str>,
        libs: &[String],
    ) -> Result<crate::rpc::BuildFullResponse, MccServerError> {
        let client = self.client().ok_or(MccServerError::NotConnected)?;
        client
            .build_full(entry, top, libs)
            .await
            .map_err(|e| {
                if e.is_connection_loss() {
                    self.mark_dead();
                }
                MccServerError::Rpc(e.to_string())
            })
    }

    /// Find mcc binary path
    fn find_mcc_path() -> PathBuf {
        // Check MCC_PATH env var first
        if let Ok(path) = std::env::var("MCC_PATH") {
            let p = PathBuf::from(&path);
            if p.exists() {
                debug!("Found mcc via MCC_PATH: {:?}", p);
                return p;
            }
            warn!("MCC_PATH={} does not exist, falling back", path);
        }

        // Try common relative locations (cargo workspace layout)
        let candidates = [
            PathBuf::from("../mcc/target/debug/mcc"),
            PathBuf::from("../../mcc/target/debug/mcc"),
            PathBuf::from("target/debug/mcc"),
        ];

        for path in &candidates {
            if path.exists() {
                debug!("Found mcc at {:?}", path);
                return path.clone();
            }
        }

        // Fallback to PATH
        debug!("Using mcc from PATH");
        PathBuf::from("mcc")
    }
}

impl Default for MccServer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum MccServerError {
    Spawn(String),
    FailedToStart(String),
    NotConnected,
    Rpc(String),
}

impl std::fmt::Display for MccServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MccServerError::Spawn(s) => write!(f, "Failed to spawn mcc server: {}", s),
            MccServerError::FailedToStart(s) => write!(f, "Failed to start mcc server: {}", s),
            MccServerError::NotConnected => write!(f, "mcc server is not connected"),
            MccServerError::Rpc(s) => write!(f, "RPC error: {}", s),
        }
    }
}

/// How a reuse attempt against a discovered daemon ended.
enum ReuseError {
    /// Drift, restart performed (§4.4) — the caller falls through to spawn.
    Restarted,
    /// Unrecoverable at the connection face — propagate.
    Fatal(MccServerError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::RpcError;
    use std::sync::atomic::Ordering;

    #[test]
    fn slot_for_project_root() {
        let root = Path::new("/tmp/proj");
        let slot = Slot::for_root(Some(root));
        assert_eq!(slot.pid_path, Path::new("/tmp/proj/.mcode/mcc.pid"));
        assert_eq!(slot.cwd.as_deref(), Some(root));
        assert!(slot.fallback.is_none());
    }

    #[test]
    fn slot_for_default_root_uses_home_and_wellknown_port() {
        // No project root, no MCC_SYSTEM_ROOT: global ~/.mcode/logs/mcc.pid
        // plus the historical :8080 well-known address.
        std::env::remove_var("MCC_SYSTEM_ROOT");
        let slot = Slot::for_root(None);
        let home = std::env::var_os("HOME").expect("HOME set in test env");
        assert_eq!(
            slot.pid_path,
            PathBuf::from(home).join(".mcode").join("logs").join("mcc.pid")
        );
        assert!(slot.cwd.is_none());
        assert_eq!(
            slot.fallback,
            Some(("127.0.0.1".to_string(), MccServer::DEFAULT_PORT))
        );
    }

    #[test]
    fn parse_version_output_full_form() {
        let (v, b) = parse_version_output("mcc 0.9.1.b4546\n").expect("parses");
        assert_eq!(v, "0.9.1");
        assert_eq!(b, 4546);
    }

    #[test]
    fn parse_version_output_rejects_garbage() {
        assert!(parse_version_output("no dots here").is_none());
        assert!(parse_version_output("").is_none());
    }

    #[test]
    fn verdict_from_reply_maps_drift_to_refuse() {
        let reply = serde_json::json!({
            "handshake": {
                "verdict": "stale_build",
                "detail": "client build 1 != server build 2",
                "restart_hint": "mcc restart",
            }
        });
        assert!(matches!(
            verdict_from_reply(&reply, false),
            HandshakeVerdict::Refuse(_)
        ));
    }

    #[test]
    fn verdict_from_reply_maps_protocol_to_fatal_and_legacy_to_accept() {
        let fatal = serde_json::json!({
            "handshake": {"verdict": "protocol_mismatch", "detail": "x"}
        });
        assert!(matches!(
            verdict_from_reply(&fatal, false),
            HandshakeVerdict::Fatal(_)
        ));
        let ok = serde_json::json!({"handshake": {"verdict": "ok", "detail": "d"}});
        assert!(matches!(
            verdict_from_reply(&ok, false),
            HandshakeVerdict::Accept
        ));
        // Legacy `server.info` fallback and a sheet without a handshake
        // field both reuse on the legacy ground.
        let legacy = serde_json::json!({"schema_version": 3});
        assert!(matches!(
            verdict_from_reply(&legacy, false),
            HandshakeVerdict::Accept
        ));
        assert!(matches!(
            verdict_from_reply(&legacy, true),
            HandshakeVerdict::Accept
        ));
    }

    #[test]
    fn pid_record_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mcext-pid-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcc.pid");
        std::fs::write(&path, "1234\n127.0.0.1:54321\n").unwrap();
        let rec = read_pid_record(&path).expect("parses");
        assert_eq!(rec.pid, 1234);
        assert_eq!(rec.host, "127.0.0.1");
        assert_eq!(rec.port, 54321);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pid_record_rejects_malformed() {
        let dir = std::env::temp_dir().join(format!("mcext-pid-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcc.pid");
        std::fs::write(&path, "not-a-pid\n").unwrap();
        assert!(read_pid_record(&path).is_none());
        assert!(read_pid_record(&dir.join("missing.pid")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dead_pid_is_not_alive() {
        // PID 0 is the scheduler on unix — never signalable by us; a huge
        // pid is almost certainly unused. Either way `kill -0` must not
        // report a *running mcc* that does not exist.
        assert!(!process_alive(u32::MAX - 1));
    }

    #[test]
    fn dead_flag_defaults_false() {
        let s = MccServer::new();
        assert!(!s.dead.load(Ordering::Relaxed));
    }

    #[test]
    fn connection_loss_reads_as_disconnected() {
        let mut s = MccServer::new();
        s.state = ConnectionState::Connected;
        assert!(s.is_connected());
        s.mark_dead();
        assert!(s.connection_lost());
        assert!(!s.is_connected());
        assert!(s.client().is_none());
        // mark_dead is idempotent and logs once, but the mark stays.
        s.mark_dead();
        assert!(s.connection_lost());
        // clear_dead (start() entry) wipes the mark.
        s.clear_dead();
        assert!(s.is_connected());
    }

    #[test]
    fn reconnect_latch_fires_exactly_once() {
        let s = MccServer::new();
        assert!(s.schedule_reconnect());
        assert!(!s.schedule_reconnect());
        s.clear_reconnect_latch();
        assert!(s.schedule_reconnect());
    }

    #[test]
    fn connection_loss_distinguishes_transport_from_http() {
        assert!(RpcError::Network("connection refused".into()).is_connection_loss());
        assert!(!RpcError::Http(500, "boom".into()).is_connection_loss());
        assert!(!RpcError::Parse("bad json".into()).is_connection_loss());
        assert!(!RpcError::Server(-32601, "no such method".into()).is_connection_loss());
        assert!(!RpcError::NoResult.is_connection_loss());
    }
}
