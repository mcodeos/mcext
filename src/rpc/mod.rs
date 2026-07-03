//! RPC client for mcc server
//!
//! Sends JSON-RPC requests to `mcc server` subprocess.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// RPC client for mcc server
#[derive(Clone)]
pub struct MccRpcClient {
    base_url: String,
    client: reqwest::Client,
}

impl MccRpcClient {
    /// Create a new client connecting to the given host:port
    pub fn new(host: &str, port: u16) -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("failed to build reqwest client");
        Self {
            base_url: format!("http://{}:{}/rpc", host, port),
            client,
        }
    }

    /// Call an RPC method with params
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: Some(params),
            id: Some(serde_json::json!(1)),
        };

        let resp = self
            .client
            .post(&self.base_url)
            .json(&request)
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
        let result = self.call("library.list", json!({})).await?;
        serde_json::from_value(result).map_err(|e| RpcError::Parse(e.to_string()))
    }

    /// Get library info
    pub async fn lib_show(&self, name: &str) -> Result<LibShowResponse, RpcError> {
        let result = self.call("library.show", json!({"name": name})).await?;
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolEntry {
    pub name: String,
    pub uri: String,
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

#[derive(Debug, Clone, Deserialize)]
pub struct LapperEntry {
    pub kind: String,
    pub start: usize,
    pub stop: usize,
    pub id: u32,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GlobalSymbols {
    #[serde(default)]
    pub declares: Vec<GlobalDeclare>,
    #[serde(default)]
    pub references: Vec<GlobalReference>,
    /// ★ LSP: Cross-file goto targets (reference_id -> target)
    #[serde(default)]
    pub cross_file_targets: Vec<CrossFileTarget>,
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

/// ★ LSP: Cross-file goto target entry
#[derive(Debug, Clone, Deserialize)]
pub struct CrossFileTarget {
    pub ref_id: u32,
    pub target_uri: String,
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
                    {"kind": "class_definition", "start": 0, "stop": 9, "id": 0}
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
}
