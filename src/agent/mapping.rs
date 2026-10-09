//! caps → LLM function-calling tool mapping.
//!
//! Single-source law: this module consumes the caps JSON (`features.ai`
//! slice) and nothing else — method names, param shapes and docs come from
//! the one registry table on the server side. Adding a method to the caps
//! ai face makes it (read faces) appear here without a code change; the
//! write faces stay behind the explicit [`AgentConfig::write_face`] switch.

use serde_json::{json, Map, Value};

/// One function-calling tool as the wire wants it, plus the rpc method the
/// session dispatches to (the caps method name itself — the mcp tool names
/// carry that transport's rename/preset adaptations, not this channel's).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Methods that mutate persisted state. Never mapped unless the config
/// opens the write face; the list is the closed gate, everything else in
/// the ai slice is a read face.
const WRITE_FACE_METHODS: [&str; 3] = ["severity.set", "allow.add", "accept"];

/// The handshake method is loop-internal (the session consumes it at
/// construction), not something the model should call.
const HANDSHAKE_METHODS: [&str; 1] = ["caps"];

/// Project the caps `features.ai` slice into function-calling tool defs.
/// Returns them in caps order — the registry order is stable, so the tool
/// list presented to the model is stable too.
pub fn tools_from_caps(caps: &Value, write_face: bool) -> Vec<ToolDef> {
    let Some(contracts) = caps
        .pointer("/features/ai/contracts")
        .and_then(|c| c.as_array())
    else {
        return Vec::new();
    };
    contracts
        .iter()
        .filter_map(|row| {
            let method = row.get("method")?.as_str()?;
            if HANDSHAKE_METHODS.contains(&method) {
                return None;
            }
            if !write_face && WRITE_FACE_METHODS.contains(&method) {
                return None;
            }
            let description = row
                .pointer("/faces/mcp/0/description")
                .and_then(|d| d.as_str())
                .unwrap_or(method)
                .to_string();
            Some(ToolDef {
                name: method.to_string(),
                description,
                parameters: params_schema(row.get("params")),
            })
        })
        .collect()
}

/// Param rows (`name/kind/required/doc`) → a JSON-schema object. The kind
/// vocabulary is the one the caps face already documents (string, u32,
/// bool, object, array); an unknown kind degrades to `string` rather than
/// dropping the parameter — the model sees the doc either way.
fn params_schema(params: Option<&Value>) -> Value {
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();
    if let Some(list) = params.and_then(|p| p.as_array()) {
        for p in list {
            let (Some(name), Some(kind)) = (p.get("name"), p.get("kind")) else {
                continue;
            };
            let schema = json!({
                "type": scalar_kind(kind.as_str().unwrap_or("string")),
                "description": p.get("doc").and_then(|d| d.as_str()).unwrap_or(""),
            });
            properties.insert(name.as_str().unwrap_or("").to_string(), schema);
            if p.get("required").and_then(|r| r.as_bool()).unwrap_or(false) {
                required.push(json!(name.as_str().unwrap_or("")));
            }
        }
    }
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), Value::Array(required));
    }
    Value::Object(schema)
}

fn scalar_kind(kind: &str) -> &'static str {
    match kind {
        "u32" | "integer" => "integer",
        "bool" => "boolean",
        "object" => "object",
        _ => "string",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A trimmed caps `features.ai` slice with the real row shapes (method,
    /// params, verdict, faces) — the mapping must consume this shape, not a
    /// private one. Kept inline: the live face is exercised by the adapter
    /// smoke path, this lock only has to pin the projection rules.
    fn fixture_caps() -> Value {
        json!({
            "features": { "ai": { "contracts": [
                { "method": "check", "verdict": "fail_on_errors_strict",
                  "params": [
                    {"name": "entry", "kind": "string", "required": false, "doc": "file path"},
                    {"name": "content", "kind": "string", "required": false, "doc": "inline snippet"},
                    {"name": "strict", "kind": "bool", "required": false, "doc": "strict face"},
                    {"name": "code", "kind": "u32", "required": false, "doc": "error code"}],
                  "faces": {"mcp": [{"tool": "mcc_check_file", "description": "Check a file; returns diagnostics"}]} },
                { "method": "explain", "verdict": "known_code",
                  "params": [{"name": "code", "kind": "u32", "required": false, "doc": "code"}],
                  "faces": {"mcp": [{"tool": "mcc_explain", "description": "Explain a diagnostic code"}]} },
                { "method": "rule.detail", "verdict": "readout",
                  "params": [{"name": "code", "kind": "string", "required": true, "doc": "rule code"}],
                  "faces": {"mcp": [{"tool": "mcc_rule_detail", "description": "Rule detail"}]} },
                { "method": "severity.set", "verdict": "readout",
                  "params": [{"name": "severity", "kind": "string", "required": true, "doc": "new severity"}],
                  "faces": {"mcp": [{"tool": "mcc_severity_set", "description": "Set a severity"}]} },
                { "method": "caps", "verdict": "handshake", "params": [], "faces": {} }
            ]}}
        })
    }

    #[test]
    fn mapping__read_face_only_by_default_and_handshake_stays_internal() {
        let tools = tools_from_caps(&fixture_caps(), false);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["check", "explain", "rule.detail"]);
    }

    #[test]
    fn mapping__write_face_switch_maps_the_gated_methods() {
        let tools = tools_from_caps(&fixture_caps(), true);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["check", "explain", "rule.detail", "severity.set"]);
        assert!(
            !names.contains(&"caps"),
            "the handshake method never becomes a model-callable tool"
        );
    }

    #[test]
    fn mapping__schema_kinds_and_required_carry_over() {
        let tools = tools_from_caps(&fixture_caps(), false);
        let check = tools.iter().find(|t| t.name == "check").unwrap();
        let props = check.parameters["properties"].as_object().unwrap();
        assert_eq!(props["strict"]["type"], json!("boolean"));
        assert_eq!(props["code"]["type"], json!("integer"));
        let detail = tools.iter().find(|t| t.name == "rule.detail").unwrap();
        assert_eq!(
            detail.parameters["required"],
            json!(["code"]),
            "required params land in the schema's required list"
        );
        assert_eq!(
            check.description, "Check a file; returns diagnostics",
            "description comes from the same registry doc, not a local copy"
        );
    }

    #[test]
    fn mapping__caps_without_ai_slice_maps_to_nothing() {
        assert!(tools_from_caps(&json!({}), false).is_empty());
    }
}
