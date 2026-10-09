//! OpenAI-compatible chat-completions wire (non-streaming).
//!
//! Protocol-only by ruling: the adapter knows the request/response shape,
//! not the implementation behind it — any intranet endpoint speaking the
//! OpenAI-compatible protocol (vLLM / Ollama / llama.cpp servers, gateways)
//! works by configuration. No cloud fallback exists on this path.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::config::AgentConfig;
use super::mapping::ToolDef;

/// One conversation message in the adapter's neutral shape (converted to
/// the endpoint's wire format on the way out).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Set on `role: "tool"` messages — echoes the call the result answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn tool_result(call_id: &str, payload: String) -> Self {
        Self {
            role: "tool".into(),
            content: Some(payload),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.to_string()),
        }
    }

    /// Assistant turn as history (round-trips through serde).
    pub fn assistant(
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content,
            tool_calls,
            tool_call_id: None,
        }
    }
}

/// One tool invocation the model asked for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// JSON-encoded arguments object (the wire's spelling; parsed at dispatch).
    pub arguments: String,
}

/// What the model came back with this round: prose, tool calls, or both.
#[derive(Debug)]
pub struct AssistantTurn {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

/// Error surface of the adapter (config problems surface at construction).
#[derive(Debug)]
pub enum AgentError {
    Http(reqwest::Error),
    /// The endpoint answered but not with the expected shape.
    Protocol(String),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentError::Http(e) => write!(f, "inference endpoint unreachable: {e}"),
            AgentError::Protocol(e) => write!(f, "inference endpoint protocol: {e}"),
        }
    }
}

impl std::error::Error for AgentError {}

/// OpenAI-compatible client over one configured endpoint.
pub struct Adapter {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl Adapter {
    pub fn new(cfg: &AgentConfig) -> Result<Self, AgentError> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(AgentError::Http)?;
        Ok(Self {
            http,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            model: cfg.model.clone(),
            api_key: cfg.api_key.clone(),
        })
    }

    /// One chat round. `tools` is presented every round; the model answers
    /// with prose, tool calls, or both.
    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDef],
    ) -> Result<AssistantTurn, AgentError> {
        let wire_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "tool_choice": "auto",
        });
        if !wire_tools.is_empty() {
            body["tools"] = Value::Array(wire_tools);
        }

        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.map_err(AgentError::Http)?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AgentError::Protocol(format!(
                "chat/completions answered {status}: {}",
                truncate(&text, 400)
            )));
        }

        let payload: Value = resp.json().await.map_err(|e| {
            AgentError::Protocol(format!("response is not JSON: {e}"))
        })?;
        let message = payload
            .pointer("/choices/0/message")
            .cloned()
            .ok_or_else(|| {
                AgentError::Protocol(format!(
                    "no choices[0].message in response: {}",
                    truncate(&payload.to_string(), 400)
                ))
            })?;
        Ok(AssistantTurn {
            content: message.get("content").and_then(|c| c.as_str()).map(String::from),
            tool_calls: message
                .get("tool_calls")
                .and_then(|t| t.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|c| serde_json::from_value(c.clone()).ok())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

fn truncate(s: &str, n: usize) -> &str {
    match s.get(..n) {
        Some(head) => head,
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message__wire_shape_round_trips_tool_roles() {
        let msg = ChatMessage::tool_result("call-1", r#"{"ok":true}"#.into());
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["role"], json!("tool"));
        assert_eq!(v["tool_call_id"], json!("call-1"));
        assert!(v.get("tool_calls").is_none(), "empty tool_calls stay off the wire");

        let assistant = ChatMessage::assistant(
            None,
            vec![ToolCall {
                id: "call-2".into(),
                function: FunctionCall {
                    name: "check".into(),
                    arguments: "{}".into(),
                },
            }],
        );
        let v = serde_json::to_value(&assistant).unwrap();
        assert_eq!(v["tool_calls"][0]["function"]["name"], json!("check"));
    }
}
