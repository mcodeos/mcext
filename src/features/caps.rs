//! The kernel capability sheet — what the handshake negotiated.
//!
//! The handshake verdict alone is not the negotiation. `caps` answers with
//! the kernel's own sheet: its protocol, version, build, the method table,
//! the capability table and the feature slices. Before this module the
//! sheet was read and dropped — only the verdict survived — so the
//! negotiated capabilities were nowhere the plugin could look them up
//! (U740 leg 2: the negotiation result must land on a readable face).
//!
//! The sheet is the kernel's truth about itself; the plugin reads it, never
//! re-derives it.

use serde_json::Value;

/// The negotiated kernel capability sheet.
#[derive(Debug, Clone, Default)]
pub struct KernelCaps {
    /// The kernel that answered (`mcc`).
    pub server: String,
    /// Wire protocol tag.
    pub protocol: String,
    /// The CLI contract tag (the command line may move without the wire moving).
    pub cli_contract: String,
    /// Kernel version (`x.y.z`).
    pub version: String,
    /// Kernel build number — revision tokens are build-scoped.
    pub build: u64,
    /// The sheet's own schema version.
    pub schema_version: u64,
    /// Every method the kernel answers to.
    pub methods: Vec<String>,
    /// The capability table's names.
    pub capabilities: Vec<String>,
    /// The feature slices (diagnostics, completion, …), raw.
    pub features: Value,
    /// The handshake verdict (`ok` / `unverified` / …); empty on the legacy path.
    pub verdict: String,
    /// The verdict's detail, when it carries one.
    pub detail: Option<String>,
    /// True when the sheet came from the `server.info` fallback — an older
    /// kernel that predates `caps`; the sheet is then not a capability sheet.
    pub legacy: bool,
}

impl KernelCaps {
    /// Read a capability sheet out of a `caps` reply.
    ///
    /// `legacy` marks the `server.info` fallback: there is no sheet there,
    /// only the legacy ground record.
    pub fn from_reply(reply: &Value, legacy: bool) -> Self {
        let s = |key: &str| -> String {
            reply
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let list = |key: &str| -> Vec<String> {
            reply
                .get(key)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let handshake = reply.get("handshake");
        Self {
            server: s("server"),
            protocol: s("protocol"),
            cli_contract: s("cli_contract"),
            version: s("version"),
            build: reply.get("build").and_then(|v| v.as_u64()).unwrap_or(0),
            schema_version: reply
                .get("schema_version")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            methods: list("methods"),
            capabilities: list("capabilities"),
            features: reply.get("features").cloned().unwrap_or(Value::Null),
            verdict: handshake
                .and_then(|h| h.get("verdict"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            detail: handshake
                .and_then(|h| h.get("detail"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            legacy,
        }
    }

    /// Does the kernel answer to this method?
    pub fn has_method(&self, name: &str) -> bool {
        self.methods.iter().any(|m| m == name)
    }

    /// Does the kernel carry this capability?
    pub fn has_capability(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c == name)
    }

    /// Read a feature slice by path (`["diagnostics", "suggestions"]`).
    pub fn feature(&self, path: &[&str]) -> Option<&Value> {
        let mut cur = &self.features;
        for key in path {
            cur = cur.get(*key)?;
        }
        Some(cur)
    }

    /// One line describing the negotiated sheet — for logs and the status bar.
    pub fn summary(&self) -> String {
        if self.legacy {
            return format!(
                "legacy kernel (no capability sheet), version {}",
                if self.version.is_empty() {
                    "unknown"
                } else {
                    &self.version
                }
            );
        }
        format!(
            "{} {} (build {}, protocol {}) · {} methods · {} capabilities",
            if self.server.is_empty() {
                "mcc"
            } else {
                &self.server
            },
            self.version,
            self.build,
            self.protocol,
            self.methods.len(),
            self.capabilities.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_sheet_out_of_a_caps_reply() {
        let reply = json!({
            "server": "mcc",
            "protocol": "mcode.rpc/1",
            "cli_contract": "mcode.cli/1",
            "version": "0.9.1",
            "build": 4546,
            "schema_version": 1,
            "methods": ["completion", "sem", "caps"],
            "capabilities": ["project.index", "project.solution"],
            "features": {"diagnostics": {"suggestions": true}},
            "handshake": {"verdict": "ok"}
        });
        let caps = KernelCaps::from_reply(&reply, false);
        assert_eq!(caps.version, "0.9.1");
        assert_eq!(caps.build, 4546);
        assert!(caps.has_method("completion"));
        assert!(caps.has_capability("project.index"));
        assert!(!caps.has_method("nope"));
        assert_eq!(caps.feature(&["diagnostics", "suggestions"]), Some(&json!(true)));
        assert_eq!(caps.verdict, "ok");
        assert!(caps.summary().contains("0.9.1"));
    }

    #[test]
    fn the_legacy_fallback_is_marked_as_such() {
        let caps = KernelCaps::from_reply(&json!({"version": "0.9.0"}), true);
        assert!(caps.legacy);
        assert!(caps.methods.is_empty());
        assert!(caps.summary().contains("legacy"));
    }

    #[test]
    fn a_missing_field_is_not_an_error() {
        let caps = KernelCaps::from_reply(&json!({}), false);
        assert_eq!(caps.version, "");
        assert_eq!(caps.build, 0);
        assert!(caps.methods.is_empty());
    }
}
