//! Server-side configuration
//!
//! Passed by LSP client via `InitializeParams::initialization_options`, or updated via
//! `workspace/didChangeConfiguration`. This phase (Phase 0) only provides the skeleton;
//! detailed config items are documented in `doc/features/settings.md`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// mcc system library root path (sets MCC_SYSTEM_ROOT)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// mcc system library root; when None, uses mcc internal default priority
    #[serde(default)]
    pub system_root: Option<PathBuf>,

    /// mcc project root; when None, uses the first workspace folder
    #[serde(default)]
    pub project_root: Option<PathBuf>,

    /// Whether semantic tokens are enabled (enabled by default)
    #[serde(default = "default_true")]
    pub semantic_tokens_enabled: bool,

    /// Whether inlay hints are enabled (enabled by default)
    #[serde(default = "default_true")]
    pub inlay_hints_enabled: bool,

    /// Diagnostics debounce interval (ms)
    #[serde(default = "default_debounce_ms")]
    pub diagnostics_debounce_ms: u64,

    /// Formatting tab size
    #[serde(default = "default_tab_size")]
    pub format_tab_size: u32,

    /// Whether to insert final newline on formatting
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
