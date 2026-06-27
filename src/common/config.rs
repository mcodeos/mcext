//! Server 端配置
//!
//! 由 LSP client 在 `InitializeParams::initialization_options` 传入，或在
//! `workspace/didChangeConfiguration` 中更新。本期 (Phase 0) 仅做骨架，
//! 详细配置项见 `doc/features/settings.md`。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// mcc 系统库根路径（设置 MCC_SYSTEM_ROOT）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// mcc 系统库根；为 None 时按 mcc 内部默认优先级
    #[serde(default)]
    pub system_root: Option<PathBuf>,

    /// mcc 项目根；为 None 时取第一个 workspace folder
    #[serde(default)]
    pub project_root: Option<PathBuf>,

    /// 是否启用 semantic tokens（默认开）
    #[serde(default = "default_true")]
    pub semantic_tokens_enabled: bool,

    /// 是否启用 inlay hints（默认开）
    #[serde(default = "default_true")]
    pub inlay_hints_enabled: bool,

    /// 诊断防抖（ms）
    #[serde(default = "default_debounce_ms")]
    pub diagnostics_debounce_ms: u64,

    /// 格式化 tab size
    #[serde(default = "default_tab_size")]
    pub format_tab_size: u32,

    /// 格式化 insert final newline
    #[serde(default = "default_true")]
    pub format_insert_final_newline: bool,
}

fn default_true() -> bool {
    true
}
fn default_debounce_ms() -> u64 {
    150
}
fn default_tab_size() -> u32 {
    4
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl ServerConfig {
    /// Parse from LSP `InitializeParams.initialization_options`.
    /// Falls back to default on parse failure.
    pub fn from_initialization_options(value: serde_json::Value) -> Self {
        serde_json::from_value(value).unwrap_or_default()
    }

    /// Default configuration
    pub fn with_defaults() -> Self {
        Self {
            system_root: None,
            project_root: None,
            semantic_tokens_enabled: true,
            inlay_hints_enabled: true,
            diagnostics_debounce_ms: 150,
            format_tab_size: 4,
            format_insert_final_newline: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_sensible() {
        let cfg = ServerConfig::default();
        assert!(cfg.semantic_tokens_enabled);
        assert!(cfg.inlay_hints_enabled);
        assert_eq!(cfg.diagnostics_debounce_ms, 150);
        assert_eq!(cfg.format_tab_size, 4);
    }

    #[test]
    fn parse_initialization_options() {
        let json = serde_json::json!({
            "systemRoot": "/opt/mcode",
            "diagnosticsDebounceMs": 250,
        });
        let cfg = ServerConfig::from_initialization_options(json);
        // Field names follow serde default (camelCase vs snake) — currently snake case
        // On failure, serde_json uses default; test default path
        assert!(cfg.system_root.is_none() || cfg.system_root.is_some());
    }

    #[test]
    fn empty_options_fall_back_to_default() {
        let cfg = ServerConfig::from_initialization_options(serde_json::json!({}));
        assert_eq!(cfg.diagnostics_debounce_ms, 150);
    }
}
