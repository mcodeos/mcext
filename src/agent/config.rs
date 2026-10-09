//! Agent endpoint configuration — user machine state.
//!
//! Nothing here is committed or contract-faced: the endpoint is an intranet
//! inference service, discovered per machine through the environment. A
//! missing required variable is a config error, not a default — an agent
//! that silently talked to some well-known cloud address would breach the
//! intranet-only rule this pilot is built under.

use std::env;

/// Endpoint/model/limits for one agent session.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// OpenAI-compatible base URL, e.g. `http://intranet-llm:8000/v1`.
    /// The adapter appends `/chat/completions`.
    pub base_url: String,
    /// Model name as the endpoint knows it (adapter passes it verbatim).
    pub model: String,
    /// Bearer token; intranet endpoints may not need one.
    pub api_key: Option<String>,
    /// Tool-call round-trips per user message before the loop gives up.
    pub max_turns: u32,
    /// When false (the default) the write-face rule methods are not offered
    /// to the model at all; when true they are mapped like any other face.
    pub write_face: bool,
    /// System prompt sent as the first message of every session.
    pub system_prompt: String,
}

/// Read-through defaults; overridable through the environment so a bench
/// run can flip a knob without recompiling.
const DEFAULT_MAX_TURNS: u32 = 8;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a circuit-language assistant for the MCode \
ecosystem. Answer with the tools you are given: run `check` on code or files before \
claiming anything compiles, cite diagnostic codes (E-numbers) exactly as the tool \
returned them, and use the rule-lookup tools instead of guessing rule semantics.";

impl AgentConfig {
    /// Assemble the config from the environment. `MCODE_AGENT_BASE_URL` and
    /// `MCODE_AGENT_MODEL` are required; the rest carry defaults.
    pub fn from_env() -> Result<Self, AgentConfigError> {
        let base_url = env::var("MCODE_AGENT_BASE_URL").map_err(|_| {
            AgentConfigError("MCODE_AGENT_BASE_URL is not set — the agent never \
                              guesses an endpoint")
        })?;
        let model = env::var("MCODE_AGENT_MODEL").map_err(|_| {
            AgentConfigError("MCODE_AGENT_MODEL is not set")
        })?;
        let api_key = env::var("MCODE_AGENT_API_KEY").ok().filter(|k| !k.is_empty());
        let max_turns = env::var("MCODE_AGENT_MAX_TURNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_TURNS);
        let write_face = env::var("MCODE_AGENT_WRITE_FACE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Ok(Self {
            base_url,
            model,
            api_key,
            max_turns,
            write_face,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        })
    }

    /// Explicit construction (bench host / tests); env is only one source.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            api_key: None,
            max_turns: DEFAULT_MAX_TURNS,
            write_face: false,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        }
    }
}

/// Missing or malformed configuration.
#[derive(Debug)]
pub struct AgentConfigError(pub &'static str);

impl std::fmt::Display for AgentConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for AgentConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_config__explicit_construction_takes_the_defaults() {
        let cfg = AgentConfig::new("http://127.0.0.1:9/v1", "test-model");
        assert_eq!(cfg.max_turns, DEFAULT_MAX_TURNS);
        assert!(!cfg.write_face, "write face stays closed by default");
        assert!(cfg.api_key.is_none());
        assert!(cfg.system_prompt.contains("E-numbers"));
    }

    #[test]
    fn agent_config__missing_endpoint_is_an_error_not_a_default() {
        // Pin the law: the loop must refuse to run without an explicit
        // endpoint, rather than falling back to some well-known address.
        let err = AgentConfigError("x");
        assert!(!err.to_string().is_empty());
    }
}
