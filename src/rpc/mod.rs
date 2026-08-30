//! RPC client for mcc server
//!
//! Sends JSON-RPC requests to `mcc server` subprocess.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

mod types;

/// RPC client for mcc server
#[derive(Clone)]
pub struct MccRpcClient {
    base_url: String,
    client: reqwest::Client,
}

impl MccRpcClient {
    /// Create a new client connecting to the given host:port
    pub fn new(host: &str, port: u16) -> Result<Self, RpcError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| RpcError::Network(e.to_string()))?;
        Ok(Self {
            base_url: format!("http://{}:{}/rpc", host, port),
            client,
        })
    }

    /// Call an RPC method with params
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_with_timeout(method, params, None).await
    }

    /// Call an RPC method with an optional per-request timeout override.
    ///
    /// The client's default request timeout is 60s (set in [`MccRpcClient::new`]).
    /// Long-running methods like `build.full` pass an explicit longer timeout so
    /// a whole-project build isn't cut off mid-pass.
    pub async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Option<std::time::Duration>,
    ) -> Result<Value, RpcError> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: Some(params),
            id: Some(serde_json::json!(1)),
        };

        let mut req = self.client.post(&self.base_url).json(&request);
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| RpcError::Network(e.to_string()))?;

        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| RpcError::Parse(format!("bytes error: {}", e)))?;
        let body = String::from_utf8_lossy(&bytes).to_string();

        if !status.is_success() {
            return Err(RpcError::Network(format!(
                "{} {}: {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("unknown"),
                body
            )));
        }

        let json: JsonRpcResponse = serde_json::from_str(&body)
            .map_err(|e| RpcError::Parse(format!("{}: body='{}'", e, &body)))?;

        if let Some(err) = json.error {
            return Err(RpcError::Server(err.code, err.message));
        }

        json.result.ok_or(RpcError::NoResult)
    }

    /// Get semantic data (tokens + symbols) for a file
    pub async fn sem(&self, uri: &str, content: Option<&str>) -> Result<SemResponse, RpcError> {
        let params = if let Some(text) = content {
            json!({"uri": uri, "content": text})
        } else {
            json!({"uri": uri})
        };
        let result = self.call("sem", params).await?;
        serde_json::from_value(result).map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Parse a project/file
    pub async fn parse(&self, entry: &str) -> Result<Value, RpcError> {
        self.call("parse", json!({"entry": entry, "include_system": true}))
            .await
    }

    /// Get diagnostics for a file
    pub async fn diagnostics(&self, uri: &str) -> Result<DiagnosticsResponse, RpcError> {
        let result = self.call("diagnostics", json!({"uri": uri})).await?;
        serde_json::from_value(result).map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Get project-wide symbols (components, interfaces, enums, modules)
    pub async fn project_symbols(&self) -> Result<ProjectSymbolsResponse, RpcError> {
        let result = self.call("project_symbols", json!({})).await?;
        serde_json::from_value(result).map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Set project root
    pub async fn set_project_root(&self, path: &str) -> Result<(), RpcError> {
        self.call("set_project_root", json!({"path": path})).await?;
        Ok(())
    }

    /// Set system root (for library resolution)
    pub async fn set_system_root(&self, path: &str) -> Result<(), RpcError> {
        self.call("set_system_root", json!({"path": path})).await?;
        Ok(())
    }

    /// Initialize mcc system
    pub async fn init(&self) -> Result<(), RpcError> {
        self.call("init", json!({})).await?;
        Ok(())
    }

    /// Load project
    pub async fn load_project(&self, entry: &str) -> Result<(), RpcError> {
        self.call("load_project", json!({"entry": entry})).await?;
        Ok(())
    }

    /// Add file to project
    pub async fn add_file(&self, uri: &str) -> Result<(), RpcError> {
        self.call("add_file", json!({"uri": uri})).await?;
        Ok(())
    }

    /// Remove file from project
    pub async fn remove_file(&self, uri: &str) -> Result<(), RpcError> {
        self.call("remove_file", json!({"uri": uri})).await?;
        Ok(())
    }

    /// Load a library by name
    pub async fn lib_load(&self, name: &str) -> Result<(), RpcError> {
        self.call("lib.load", json!({"name": name})).await?;
        Ok(())
    }

    /// List loaded libraries
    pub async fn lib_list(&self) -> Result<LibListResponse, RpcError> {
        let result = self.call("lib.list", json!({})).await?;
        serde_json::from_value(result).map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Get library info
    pub async fn lib_show(&self, name: &str) -> Result<LibShowResponse, RpcError> {
        let result = self.call("lib.info", json!({"name": name})).await?;
        serde_json::from_value(result).map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Build + render viz to a self-contained HTML string (circuit viewer).
    ///
    /// `libs` are pre-loaded by mcc via `lib.load` during init, but passing them
    /// explicitly keeps `build.viz` deterministic (mirrors `mcc build --viz`).
    pub async fn build_viz(
        &self,
        entry: &str,
        top: Option<&str>,
        libs: &[String],
        layouter: Option<&str>,
    ) -> Result<String, RpcError> {
        let mut params = json!({"entry": entry, "libs": libs, "include_system": true});
        if let Some(t) = top {
            params["top"] = json!(t);
        }
        if let Some(l) = layouter {
            params["layouter"] = json!(l);
        }
        let result = self.call("build.viz", params).await?;
        result
            .get("html")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| RpcError::Parse("build.viz response missing 'html'".into()))
    }

    /// Build the whole project (equivalent of `mcc build`), returning the
    /// structured per-phase envelope (pass0/pass1/pass2 diagnostics + summary).
    ///
    /// `libs` are pre-loaded by mcc via `lib.load` during init, but passing them
    /// explicitly keeps `build.full` deterministic (mirrors `mcc build`). A full
    /// project build can exceed the default 60s request timeout, so this uses a
    /// 300s per-request timeout.
    pub async fn build_full(
        &self,
        entry: &str,
        top: Option<&str>,
        libs: &[String],
    ) -> Result<BuildFullResponse, RpcError> {
        let mut params = json!({"entry": entry, "libs": libs, "include_system": true});
        if let Some(t) = top {
            params["top"] = json!(t);
        }
        let result = self
            .call_with_timeout(
                "build.full",
                params,
                Some(std::time::Duration::from_secs(300)),
            )
            .await?;
        serde_json::from_value(result).map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Layered completion for a cursor position (design §8.1).
    ///
    /// When `member_root` is present (e.g. `uC` or `this`), mcc returns the
    /// `Member` layer instead of P1-P5 (§5.6).
    pub async fn completion(
        &self,
        uri: &str,
        position: usize,
        prefix: Option<&str>,
        member_root: Option<&str>,
    ) -> Result<CompletionResponse, RpcError> {
        let mut params = json!({"uri": uri, "position": position});
        if let Some(p) = prefix {
            params["prefix"] = json!(p);
        }
        if let Some(m) = member_root {
            params["member_root"] = json!(m);
        }
        let result = self.call("completion", params).await?;
        serde_json::from_value(result).map_err(|e| RpcError::Parse(e.to_string()))
    }
}

/// Response from `diagnostics` RPC
#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticsResponse {
    pub diagnostics: Vec<DiagEntry>,
}

/// Response from `library.list` RPC
#[derive(Debug, Clone, Deserialize)]
pub struct LibListResponse {
    pub loaded: Vec<LibEntry>,
    pub installed: Vec<LibEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibEntry {
    pub name: String,
}

/// Response from `library.show` RPC
#[derive(Debug, Clone, Deserialize)]
pub struct LibShowResponse {
    pub name: String,
    pub total_symbols: usize,
    pub module_count: usize,
    pub component_count: usize,
    pub interface_count: usize,
}

/// Response from `project_symbols` RPC
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectSymbolsResponse {
    pub components: Vec<SymbolEntry>,
    pub interfaces: Vec<SymbolEntry>,
    pub enums: Vec<SymbolEntry>,
    pub modules: Vec<SymbolEntry>,
    /// ★ enum value rows: one per `enum Foo { VALUE }` body row.
    /// Addresses this: jump-to-definition needs (class, value) -> span.
    #[serde(default)]
    pub enum_values: Vec<EnumValueEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolEntry {
    pub name: String,
    pub uri: String,
    /// Byte span [start, end) of the class head (e.g. `enum PKG { ... }`).
    /// Older mcc servers may omit this — default to [0, 0] in that case.
    #[serde(default)]
    pub span: [usize; 2],
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnumValueEntry {
    /// Owning enum class name (e.g. "PKG").
    pub class: String,
    /// Value name (e.g. "SOP8").
    pub name: String,
    pub uri: String,
    /// Byte span [start, end) of the value row (e.g. `SOP8,` inside the body).
    #[serde(default)]
    pub span: [usize; 2],
}

/// Response from `completion` RPC with a cursor position (layered, §8.1).
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionResponse {
    /// P1..P5 path of the cursor, e.g. "US513.i2c".
    pub scope_path: String,
    /// Layer name → candidates. mcc omits layers with no items.
    pub layers: HashMap<String, Vec<CompletionLayerItem>>,
    /// Layers truncated at the per-layer cap (§8.5).
    #[serde(default)]
    pub truncated_layers: Vec<String>,
}

/// One layered completion candidate (§8.1).
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionLayerItem {
    pub name: String,
    /// Symbol kind string, e.g. "function", "port", "component".
    pub kind: String,
    #[serde(default)]
    pub scope: String,
    pub uri: String,
    /// Byte span [start, end) of the def. Object form `{"start":N,"end":N}`.
    #[serde(default)]
    pub span: CompletionSpan,
}

/// Byte span of a completion candidate.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CompletionSpan {
    #[serde(default)]
    pub start: usize,
    #[serde(default)]
    pub end: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagEntry {
    pub code: u32,
    pub level: String,
    pub message: String,
    pub location: DiagLocation,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagLocation {
    pub pos: u32,
    pub len: u32,
    pub line: u32,
    pub column: u32,
}

/// Response from `build.full` RPC (equivalent of `mcc build`).
///
/// The mcc backend returns a per-phase envelope: pass0 (lib-load diagnostics),
/// pass1 (parse/definitions), pass2 (instantiation) plus a summary. Unknown
/// envelope fields are ignored by serde.
#[derive(Debug, Clone, Deserialize)]
pub struct BuildFullResponse {
    pub workspace: BuildWorkspaceInfo,
    pub pass0: BuildPass,
    pub pass1: BuildPass,
    pub pass2: BuildPass,
    pub summary: BuildSummary,
    /// Failure ledger (resolve-gate-design.md §7.1-2): cross-pass record of
    /// non-clean parses (silent fallbacks, phantoms, floating wires). Optional
    /// so a backend that doesn't emit it still deserializes.
    #[serde(default)]
    pub ledger: Option<LedgerReport>,
}

/// Failure ledger summary (resolve-gate-design.md §7.1-2): kind×form counts
/// plus per-row detail. Mirrors the mcc backend's `LedgerReport` shape.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct LedgerReport {
    pub total: usize,
    /// kind → form → count (all six kinds present; empty inner map = none).
    #[serde(default)]
    pub by_kind_form: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, usize>,
    >,
    #[serde(default)]
    pub resolved_late: usize,
    /// Per-row detail, only when the backend was asked for it (`--ledger`).
    #[serde(default)]
    pub detail: Vec<LedgerDetailRow>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct LedgerDetailRow {
    pub kind: String,
    pub form: String,
    pub site: String,
    pub action: String,
    #[serde(default)]
    pub refs: Option<u32>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub column: Option<u32>,
    #[serde(default)]
    pub pos: Option<u32>,
    #[serde(default)]
    pub len: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BuildWorkspaceInfo {
    pub kind: String,
    pub name: String,
}

/// One pipeline phase (pass0/pass1/pass2). Only the diagnostics are consumed.
#[derive(Debug, Clone, Deserialize)]
pub struct BuildPass {
    #[serde(default)]
    pub diagnostics: Vec<BuildDiag>,
}

/// One diagnostic in the build envelope. `line`/`column` are 1-based (mcc
/// `Location::row`/`col`); `file` is an `McURI` (plain path or `file://…`).
#[derive(Debug, Clone, Deserialize)]
pub struct BuildDiag {
    pub phase: String,
    pub severity: String,
    pub code: u32,
    pub message: String,
    pub location: BuildLoc,
    #[serde(default)]
    pub suggestions: Vec<serde_json::Value>,
    #[serde(default)]
    pub related: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BuildLoc {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub pos: u32,
    pub len: u32,
}

/// Build summary counters, mirroring the envelope's `summary` object.
#[derive(Debug, Clone, Deserialize)]
pub struct BuildSummary {
    pub errors: u64,
    pub warnings: u64,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub module_count: u64,
    #[serde(default)]
    pub component_count: u64,
    #[serde(default)]
    pub interface_count: u64,
    #[serde(default)]
    pub instance_count: u64,
    #[serde(default)]
    pub net_count: u64,
    /// Categorized statistics (system/project splits + used classes + instance
    /// breakdown), mirroring `mcc build`'s Summary block. Absent when the daemon
    /// is older than the build.full change that added it.
    #[serde(default)]
    pub stats: Option<BuildStats>,
}

/// Categorized build statistics — the same numbers `mcc build` renders in its
/// Summary block. `ns_*` split the *namespace classes* (all defined modules /
/// components / interfaces) by definition space; `used_*` split the classes
/// *actually instantiated*; `module_insts`/`component_insts` are the instance
/// counts by kind.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BuildStats {
    #[serde(default)]
    pub ns_modules_system: u64,
    #[serde(default)]
    pub ns_modules_project: u64,
    #[serde(default)]
    pub ns_components_system: u64,
    #[serde(default)]
    pub ns_components_project: u64,
    #[serde(default)]
    pub ns_interfaces_system: u64,
    #[serde(default)]
    pub ns_interfaces_project: u64,
    #[serde(default)]
    pub used_modules_system: u64,
    #[serde(default)]
    pub used_modules_project: u64,
    #[serde(default)]
    pub used_components_system: u64,
    #[serde(default)]
    pub used_components_project: u64,
    #[serde(default)]
    pub module_insts: u64,
    #[serde(default)]
    pub component_insts: u64,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    #[serde(rename = "jsonrpc")]
    jsonrpc: String,
    method: String,
    params: Option<Value>,
    id: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcResponse {
    #[serde(rename = "jsonrpc")]
    jsonrpc: String,
    result: Option<Value>,
    error: Option<JsonRpcErrorDetail>,
    id: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcErrorDetail {
    code: i32,
    message: String,
}

#[derive(Debug)]
pub enum RpcError {
    Network(String),
    Parse(String),
    Server(i32, String),
    NoResult,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Network(s) => write!(f, "Network error: {}", s),
            RpcError::Parse(s) => write!(f, "Parse error: {}", s),
            RpcError::Server(code, msg) => write!(f, "Server error [{}]: {}", code, msg),
            RpcError::NoResult => write!(f, "No result in response"),
        }
    }
}

impl std::error::Error for RpcError {}

/// Response from `sem` RPC
#[derive(Debug, Clone, Deserialize)]
pub struct SemResponse {
    pub tokens: Vec<SemToken>,
    pub symbols: SemSymbols,
    /// Stable result_id for semantic tokens (hash of token data)
    #[serde(default)]
    pub result_id: Option<String>,
    /// §7.6: Files that `use` this one — need re-parse
    #[serde(default)]
    pub affected_uris: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemToken {
    #[serde(rename = "type")]
    pub token_type: i16,
    pub position: i32,
    pub length: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SemSymbols {
    pub local: LocalSymbols,
    pub lapper: Vec<LapperEntry>,
    #[serde(default)]
    pub global: GlobalSymbols,
    /// ★ Unified ref→def map (RefDefMap) — replaces cross_file_targets
    #[serde(default)]
    pub ref_def_map: Option<RefDefMapData>,
}

/// RefDefMap payload from mcc sem RPC.
#[derive(Debug, Clone, Deserialize)]
pub struct RefDefMapData {
    pub entries: Vec<RefDefEntryData>,
    pub files: Vec<String>,
    pub containers: Vec<String>,
    #[serde(default)]
    pub func_names: Vec<String>,
    #[serde(default)]
    pub kind_names: Vec<String>,
    /// §7.6: Content hash for mcext dedup.
    #[serde(default)]
    pub result_id: u64,
    /// O(1) index: (ref_kind, ref_id) → entry index. Built lazily.
    #[serde(skip)]
    pub(crate) index: OnceLock<HashMap<(u8, u32), usize>>,
    /// kind name → ordinal map. Built lazily from kind_names.
    #[serde(skip)]
    pub(crate) kind_map: OnceLock<HashMap<String, u8>>,
}

/// Ref→def mapping entry — RPC wire type (see `types` module).
pub use types::RefDefEntryData;

impl RefDefMapData {
    /// O(1) lookup by (ref_kind, ref_id). Builds HashMap index on first call.
    pub fn lookup(&self, ref_kind: u8, ref_id: u32) -> Option<&RefDefEntryData> {
        let idx = self.index.get_or_init(|| {
            let mut m = HashMap::new();
            for (i, e) in self.entries.iter().enumerate() {
                m.insert((e.ref_kind, e.ref_id), i);
            }
            m
        });
        idx.get(&(ref_kind, ref_id)).map(|&i| &self.entries[i])
    }

    /// Build `kind_name → ordinal` reverse map from `kind_names`.
    pub fn kind_map(&self) -> &HashMap<String, u8> {
        self.kind_map.get_or_init(|| {
            self.kind_names
                .iter()
                .enumerate()
                .map(|(i, name)| (name.clone(), i as u8))
                .collect()
        })
    }

    /// Lookup ref_kind ordinal by lapper kind string.
    pub fn resolve_kind(&self, kind_str: &str) -> Option<u8> {
        self.kind_map().get(kind_str).copied()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalSymbols {
    pub declares: Vec<LocalDeclare>,
    #[serde(default)]
    pub references: Vec<LocalReference>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalDeclare {
    pub kind: String,
    pub id: u32,
    pub span: [usize; 2],
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalReference {
    pub kind: String,
    pub id: u32,
    pub span: [usize; 2],
    #[serde(default)]
    pub declare_id: Option<u32>,
}

/// Lapper interval entry — RPC wire type (see `types` module).
pub use types::LapperEntry;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GlobalSymbols {
    #[serde(default)]
    pub declares: Vec<GlobalDeclare>,
    #[serde(default)]
    pub references: Vec<GlobalReference>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GlobalDeclare {
    pub id: u32,
    pub uri: String,
    pub span: [usize; 2],
}

#[derive(Debug, Clone, Deserialize)]
pub struct GlobalReference {
    pub id: u32,
    pub uri: String,
    pub span: [usize; 2],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sem_response() {
        let json = serde_json::json!({
            "tokens": [
                {"type": 1, "position": 0, "length": 9}
            ],
            "symbols": {
                "local": {
                    "declares": [
                        {"kind": "declare", "id": 0, "span": [0, 9]}
                    ],
                    "references": []
                },
                "lapper": [
                    {"kind": 0, "start": 0, "stop": 9, "id": 0}
                ],
                "global": {
                    "declares": [],
                    "references": []
                }
            }
        });

        let resp: SemResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.tokens.len(), 1);
        assert_eq!(resp.symbols.lapper.len(), 1);
    }

    #[test]
    fn parse_completion_response() {
        let json = serde_json::json!({
            "scope_path": "US513.i2c",
            "layers": {
                "P2": [
                    {
                        "name": "uC",
                        "kind": "instance",
                        "scope": "US513",
                        "uri": "file:///p/us513.mc",
                        "span": {"start": 7345, "end": 7350}
                    }
                ],
                "Member": [
                    {
                        "name": "I2C0",
                        "kind": "port",
                        "scope": "US513.i2c",
                        "uri": "file:///p/us513.mc",
                        "span": {"start": 7390, "end": 7395}
                    }
                ]
            },
            "truncated_layers": ["P5"]
        });

        let resp: CompletionResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.scope_path, "US513.i2c");
        assert_eq!(resp.truncated_layers, vec!["P5"]);
        let p2 = resp.layers.get("P2").unwrap();
        assert_eq!(p2.len(), 1);
        assert_eq!(p2[0].name, "uC");
        assert_eq!(p2[0].span.start, 7345);
        let mem = resp.layers.get("Member").unwrap();
        assert_eq!(mem[0].name, "I2C0");
        assert_eq!(mem[0].span.end, 7395);
    }

    #[test]
    fn parse_build_full_response() {
        let json = serde_json::json!({
            "command": "mcc build",
            "workspace": {"kind": "project", "name": "demo"},
            "pass0": {
                "loaded_files": [],
                "diagnostics": [
                    {
                        "phase": "pass0",
                        "severity": "warning",
                        "code": 1001,
                        "message": "lib stub",
                        "location": {"file": "/sys/lib.mc", "line": 3, "column": 1, "pos": 80, "len": 4},
                        "suggestions": [],
                        "related": []
                    }
                ]
            },
            "pass1": {
                "definitions": {"modules": [], "components": [], "interfaces": []},
                "diagnostics": [
                    {
                        "phase": "pass1",
                        "severity": "error",
                        "code": 2002,
                        "message": "undefined ref",
                        "location": {"file": "/p/main.mc", "line": 12, "column": 7, "pos": 320, "len": 5},
                        "suggestions": [],
                        "related": [{"message": "here", "location": {"file": "/p/other.mc", "line": 1, "column": 1, "pos": 0, "len": 1}}]
                    }
                ]
            },
            "pass2": {"diagnostics": []},
            "summary": {
                "module_count": 1, "component_count": 2, "interface_count": 0,
                "instance_count": 10, "net_count": 5,
                "errors": 1, "warnings": 1, "elapsed_ms": 42
            }
        });

        let resp: BuildFullResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.workspace.kind, "project");
        assert_eq!(resp.pass0.diagnostics.len(), 1);
        assert_eq!(resp.pass1.diagnostics.len(), 1);
        assert!(resp.pass2.diagnostics.is_empty());
        let d = &resp.pass1.diagnostics[0];
        assert_eq!(d.phase, "pass1");
        assert_eq!(d.severity, "error");
        assert_eq!(d.code, 2002);
        assert_eq!(d.location.file, "/p/main.mc");
        assert_eq!(d.location.line, 12);
        assert_eq!(d.location.column, 7);
        assert_eq!(d.related.len(), 1);
        assert_eq!(resp.summary.errors, 1);
        assert_eq!(resp.summary.warnings, 1);
        assert_eq!(resp.summary.elapsed_ms, 42);
    }
}
