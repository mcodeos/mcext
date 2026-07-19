//! MCC server manager with subprocess and RPC
//!
//! Manages an `mcc server` subprocess:
//! - Spawns mcc server process
//! - Provides RPC client for communication
//! - Auto-restarts on crash
//!
//! This solves two problems:
//! 1. Logs visible in this process's output (vs embedded in LSP)
//! 2. Crash isolation - mcc crash only kills subprocess, not LSP

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

use crate::rpc::MccRpcClient;

/// MCC server connection state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Crashed,
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
}

impl MccServer {
    /// Default MCC server host
    pub const DEFAULT_HOST: &'static str = "127.0.0.1";
    /// Default MCC server port
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
        }
    }

    /// Create with custom host/port
    pub fn with_addr(host: &str, port: u16) -> Self {
        let mut s = Self::new();
        s.host = host.to_string();
        s.port = port;
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

    /// Start mcc server subprocess and connect
    pub async fn start(&mut self) -> Result<(), MccServerError> {
        // Clear log at start of each session
        Self::clear_log();

        if self.state == ConnectionState::Connected {
            return Ok(());
        }

        warn!("=== MccServer::start called ===");

        // Check if port is already in use
        let check_addr = format!("{}:{}", self.host, self.port);
        if std::net::TcpListener::bind(&check_addr).is_err() {
            info!("Port {} is already in use, trying to connect", self.port);
            match MccRpcClient::new(&self.host, self.port) {
                Ok(client) => {
                    match timeout(
                        Duration::from_secs(2),
                        client.call("server.info", serde_json::json!({})),
                    )
                    .await
                    {
                        Ok(Ok(_)) => {
                            self.client = Some(client);
                            self.state = ConnectionState::Connected;
                            info!("Connected to existing mcc server");
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

        // Use a loop for retries
        loop {
            self.state = ConnectionState::Connecting;
            info!(
                "Starting mcc server (attempt {}/{})...",
                self.restart_count + 1,
                self.max_restarts
            );

            // Find mcc binary
            let mcc_path = Self::find_mcc_path();

            // Start server (correct command is "mcc start")
            info!("Spawning mcc from: {:?}", mcc_path);
            let mut child = match Command::new(&mcc_path)
                .arg("start")
                .stdout(Stdio::null()) // Don't capture stdout, let mcc write directly
                .stderr(Stdio::inherit()) // Inherit stderr for debugging
                .kill_on_drop(true)
                .spawn()
            {
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

            // Check if process exits immediately
            match child.try_wait() {
                Ok(Some(status)) => {
                    error!("mcc process exited immediately: {:?}", status);
                    self.restart_count += 1;
                    if self.restart_count >= self.max_restarts {
                        return Err(MccServerError::FailedToStart(format!(
                            "mcc exited: {:?}",
                            status
                        )));
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                Ok(None) => {
                    info!("mcc is still running");
                } // Still running
                Err(e) => {
                    warn!("Failed to check mcc status: {}", e);
                }
            }

            // Try RPC connection after a brief delay
            info!("Waiting for mcc to be ready...");

            // Wait for port to be bound
            let deadline = Instant::now() + Duration::from_secs(5);
            let addr = format!("{}:{}", self.host, self.port);
            while Instant::now() < deadline {
                if std::net::TcpListener::bind(&addr).is_err() {
                    info!("Port {} is now bound, mcc is listening", self.port);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }

            // Extra wait for mcc to fully initialize
            info!("Extra wait for mcc initialization...");
            tokio::time::sleep(Duration::from_secs(1)).await;

            // Try to connect with retries
            info!("Attempting RPC connection to {}:{}", self.host, self.port);
            let client = match MccRpcClient::new(&self.host, self.port) {
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
                match timeout(
                    Duration::from_secs(2),
                    client.call("server.info", serde_json::json!({})),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        self.client = Some(client);
                        self.child = Some(child);
                        self.state = ConnectionState::Connected;
                        self.restart_count = 0;
                        info!("mcc server connected at {}:{}", self.host, self.port);
                        return Ok(());
                    }
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

    /// Get RPC client
    pub fn client(&self) -> Option<&MccRpcClient> {
        if self.state == ConnectionState::Connected {
            self.client.as_ref()
        } else {
            None
        }
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.state == ConnectionState::Connected
    }

    /// Get connection state
    pub fn state(&self) -> ConnectionState {
        self.state
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
            .map_err(|e| MccServerError::Rpc(e.to_string()))
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
            .map_err(|e| MccServerError::Rpc(e.to_string()))
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

impl std::error::Error for MccServerError {}
