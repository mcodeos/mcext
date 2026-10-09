//! The turn loop: user message → model → tool calls → rpc dispatch →
//! results → model → final text.
//!
//! The rpc seam is a trait so the loop is testable without a daemon; the
//! production impl forwards to the shared [`MccRpcClient`]. The loop knows
//! nothing about editors — no tower-lsp types cross this line (portability
//! law for the later bench-host move).

use serde_json::{json, Value};

use super::adapter::{Adapter, ChatMessage, ToolCall};
use super::config::AgentConfig;
use super::mapping::{tools_from_caps, ToolDef};

/// The loop's view of the mcc rpc channel.
pub trait AgentRpc: Send + Sync {
    fn call(
        &self,
        method: &str,
        params: Value,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send;
}

/// Production impl over the shared rpc client.
pub struct RpcDispatch<'a>(pub &'a crate::rpc::MccRpcClient);

impl AgentRpc for RpcDispatch<'_> {
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.0.call(method, params).await.map_err(|e| e.to_string())
    }
}

/// One agent session: fixed tool set (from caps at construction), growing
/// history, bounded rounds per user message.
pub struct Session<R: AgentRpc> {
    rpc: R,
    adapter: Adapter,
    tools: Vec<ToolDef>,
    max_turns: u32,
    history: Vec<ChatMessage>,
}

impl<R: AgentRpc> Session<R> {
    /// Build a session: handshake the daemon for caps, map the tool face,
    /// seed the history with the system prompt.
    pub async fn new(rpc: R, cfg: &AgentConfig) -> Result<Self, String> {
        let caps = rpc.call("caps", json!({})).await.map_err(|e| {
            format!("caps handshake failed — the loop answers nothing without it: {e}")
        })?;
        Ok(Self {
            rpc,
            adapter: Adapter::new(cfg).map_err(|e| e.to_string())?,
            tools: tools_from_caps(&caps, cfg.write_face),
            max_turns: cfg.max_turns,
            history: vec![ChatMessage::user(cfg.system_prompt.clone())],
        })
    }

    /// Run one user message to the model's final prose (or an error after
    /// `max_turns` tool rounds — the guard against a loop that never lands).
    pub async fn run(&mut self, user_message: &str) -> Result<String, String> {
        if self.tools.is_empty() {
            return Err(
                "no tools mapped from caps — refusing to run a tool-less loop".into(),
            );
        }
        self.history.push(ChatMessage::user(user_message));
        for _ in 0..self.max_turns {
            let turn = self
                .adapter
                .chat(&self.history, &self.tools)
                .await
                .map_err(|e| e.to_string())?;
            if turn.tool_calls.is_empty() {
                let text = turn.content.unwrap_or_default();
                self.history.push(ChatMessage::assistant(Some(text.clone()), vec![]));
                return Ok(text);
            }
            self.history
                .push(ChatMessage::assistant(turn.content.clone(), turn.tool_calls.clone()));
            for call in turn.tool_calls {
                let payload = self.dispatch(&call).await;
                self.history
                    .push(ChatMessage::tool_result(&call.id, payload));
            }
        }
        Err(format!(
            "no final answer after {} tool rounds — stopping instead of looping",
            self.max_turns
        ))
    }

    /// Dispatch one model-requested tool call to the rpc channel. Tool
    /// errors go back to the model as payload text — the model can correct
    /// course; only transport/loop failures are hard errors.
    async fn dispatch(&self, call: &ToolCall) -> String {
        let args: Value = match serde_json::from_str(&call.function.arguments) {
            Ok(v) => v,
            Err(e) => {
                return json!({"error": format!("arguments are not valid JSON: {e}")})
                    .to_string();
            }
        };
        match self.rpc.call(&call.function.name, args).await {
            Ok(mut result) => {
                // world_ver rides the envelopes the server stamps; keep it
                // visible at the top level so every tool answer carries the
                // freshness the model read (full T6.6 lands later).
                if let Some(ver) = result.get("world_ver").cloned() {
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert("world_ver_read".into(), ver);
                    }
                }
                result.to_string()
            }
            Err(e) => json!({ "error": e }).to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Scripted rpc: pops one canned answer per call; the loop test asserts
    /// the dispatch shapes, not the daemon (that face is covered upstream).
    struct ScriptedRpc(Mutex<VecDeque<Result<Value, String>>>);

    impl AgentRpc for ScriptedRpc {
        async fn call(&self, method: &str, _params: Value) -> Result<Value, String> {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .expect("rpc script exhausted")
                .map(|v| {
                    if method == "caps" {
                        v
                    } else {
                        json!({"echo": method, "result": v})
                    }
                })
        }
    }

    fn caps_value() -> Value {
        json!({
            "features": {"ai": {"contracts": [
                {"method": "check", "verdict": "readout",
                 "params": [{"name": "content", "kind": "string", "required": false, "doc": "snippet"}],
                 "faces": {"mcp": [{"tool": "mcc_check_file", "description": "Check a file"}]}}
            ]}}
        })
    }

    /// Minimal HTTP/1.1 server speaking just enough of the wire for reqwest:
    /// one response per connection, from a script queue.
    async fn mock_endpoint(script: Mutex<VecDeque<Value>>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let Some(resp) = script.lock().unwrap().pop_front() else { break };
                let mut buf = vec![0u8; 65536];
                let _ = sock.readable().await;
                // Read the request head (best-effort; we do not parse it).
                let _ = sock.try_read(&mut buf);
                let body = serde_json::to_string(&resp).unwrap();
                let http = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                use tokio::io::AsyncWriteExt;
                let _ = sock.write_all(http.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        (format!("http://{addr}/v1"), handle)
    }

    fn tool_call_turn(id: &str, name: &str, args: &str) -> Value {
        json!({"choices": [{"message": {
            "content": null,
            "tool_calls": [{"id": id, "type": "function",
                            "function": {"name": name, "arguments": args}}]
        }}]})
    }

    fn final_turn(text: &str) -> Value {
        json!({"choices": [{"message": {"content": text, "tool_calls": null}}]})
    }

    #[tokio::test]
    async fn session__full_loop_check_then_final_answer() {
        let rpc = ScriptedRpc(Mutex::new(VecDeque::from([
            Ok(caps_value()),                          // handshake
            Ok(json!({"diagnostics": [{"code": 2082}]})), // check call
        ])));
        let (url, server) = mock_endpoint(Mutex::new(VecDeque::from([
            tool_call_turn("c1", "check", r#"{"content": "module m{}"}"#),
            final_turn("E2082: invalid clause in a body"),
        ])))
        .await;

        let mut cfg = AgentConfig::new(url, "mock-model");
        cfg.max_turns = 4;
        let mut session = Session::new(rpc, &cfg).await.expect("session");
        let answer = session.run("what is wrong here?").await.expect("run");
        assert_eq!(answer, "E2082: invalid clause in a body");
        server.abort();
    }

    #[tokio::test]
    async fn session__rpc_error_returns_to_the_model_as_payload() {
        let rpc = ScriptedRpc(Mutex::new(VecDeque::from([
            Ok(caps_value()),
            Err("method not found".into()),
        ])));
        let (url, server) = mock_endpoint(Mutex::new(VecDeque::from([
            tool_call_turn("c1", "nonexistent.method", "{}"),
            final_turn("the tool is missing; I will say so"),
        ])))
        .await;

        let cfg = AgentConfig::new(url, "mock-model");
        let mut session = Session::new(rpc, &cfg).await.unwrap();
        let answer = session.run("go").await.unwrap();
        assert!(answer.contains("will say so"));
        server.abort();
    }

    #[tokio::test]
    async fn session__turn_guard_stops_an_endless_tool_loop() {
        // Endpoint always answers with another tool call; the session must
        // give up at max_turns instead of spinning.
        let rpc = ScriptedRpc(Mutex::new(
            std::iter::once(Ok(caps_value()))
                .chain((0..10).map(|_| Ok(json!({}))))
                .collect::<VecDeque<_>>(),
        ));
        let (url, server) = mock_endpoint(Mutex::new(
            (0..10)
                .map(|_| tool_call_turn("c", "check", "{}"))
                .collect::<VecDeque<_>>(),
        ))
        .await;

        let mut cfg = AgentConfig::new(url, "mock-model");
        cfg.max_turns = 3;
        let mut session = Session::new(rpc, &cfg).await.unwrap();
        let err = session.run("loop forever").await.unwrap_err();
        assert!(err.contains("3 tool rounds"), "guard fires: {err}");
        server.abort();
    }

    #[tokio::test]
    async fn session__refuses_to_run_tool_less() {
        let rpc = ScriptedRpc(Mutex::new(VecDeque::from([Ok(json!({}))])));
        let (url, server) = mock_endpoint(Mutex::new(VecDeque::new())).await;
        let cfg = AgentConfig::new(url, "mock-model");
        let mut session = Session::new(rpc, &cfg).await.expect("session builds");
        let err = session.run("go").await.unwrap_err();
        assert!(
            err.contains("no tools mapped from caps"),
            "empty ai slice = tool-less loop = refusal: {err}"
        );
        server.abort();
    }
}
