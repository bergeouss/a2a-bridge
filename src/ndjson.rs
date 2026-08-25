use serde::Deserialize;
use serde_json::Value;

/// Event types from Claude Code's stream-json output format
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// System initialization message
    System {
        session_id: Option<String>,
        model: Option<String>,
        subtype: Option<String>,
    },
    /// Assistant response (text + tool calls)
    Assistant {
        message: AssistantMessage,
        session_id: Option<String>,
    },
    /// Tool result (user message from tool)
    User {
        message: UserMessage,
        session_id: Option<String>,
    },
    /// Final result of the turn
    Result {
        subtype: String,
        duration_ms: Option<u64>,
        duration_api_ms: Option<u64>,
        is_error: bool,
        num_turns: Option<i64>,
        total_cost_usd: Option<f64>,
        usage: Option<Value>,
        session_id: Option<String>,
        result_text: Option<String>,
    },
    /// Raw stream event (delta content)
    StreamEvent {
        uuid: Option<String>,
        session_id: Option<String>,
    },
    /// Unknown/unparsed event
    Unknown(Value),
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: Value,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

/// Parse a single NDJSON line into a StreamEvent
pub fn parse_line(line: &str) -> Result<StreamEvent, serde_json::Error> {
    let line = line.trim();
    if line.is_empty() {
        return Err(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "empty line",
        )));
    }

    let raw: Value = serde_json::from_str(line)?;
    let event_type = raw.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "system" => {
            let session_id = raw.get("session_id").and_then(|v| v.as_str()).map(String::from);
            let model = raw.get("model").and_then(|v| v.as_str()).map(String::from);
            let subtype = raw.get("subtype").and_then(|v| v.as_str()).map(String::from);
            Ok(StreamEvent::System {
                session_id,
                model,
                subtype,
            })
        }
        "assistant" => {
            let message: AssistantMessage =
                serde_json::from_value(raw.get("message").cloned().unwrap_or_default())?;
            let session_id = raw.get("session_id").and_then(|v| v.as_str()).map(String::from);
            Ok(StreamEvent::Assistant {
                message,
                session_id,
            })
        }
        "user" => {
            let message: UserMessage =
                serde_json::from_value(raw.get("message").cloned().unwrap_or_default())?;
            let session_id = raw.get("session_id").and_then(|v| v.as_str()).map(String::from);
            Ok(StreamEvent::User {
                message,
                session_id,
            })
        }
        "result" => {
            let subtype = raw
                .get("subtype")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let duration_ms = raw.get("duration_ms").and_then(|v| v.as_u64());
            let duration_api_ms = raw.get("duration_api_ms").and_then(|v| v.as_u64());
            let is_error = raw.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
            let num_turns = raw.get("num_turns").and_then(|v| v.as_i64());
            let total_cost_usd = raw.get("total_cost_usd").and_then(|v| v.as_f64());
            let usage = raw.get("usage").cloned();
            let session_id = raw.get("session_id").and_then(|v| v.as_str()).map(String::from);
            let result_text = raw.get("result").and_then(|v| v.as_str()).map(String::from);
            Ok(StreamEvent::Result {
                subtype,
                duration_ms,
                duration_api_ms,
                is_error,
                num_turns,
                total_cost_usd,
                usage,
                session_id,
                result_text,
            })
        }
        "stream_event" => {
            let uuid = raw.get("uuid").and_then(|v| v.as_str()).map(String::from);
            let session_id = raw.get("session_id").and_then(|v| v.as_str()).map(String::from);
            Ok(StreamEvent::StreamEvent { uuid, session_id })
        }
        _ => Ok(StreamEvent::Unknown(raw)),
    }
}

/// Extract text content from an assistant message
pub fn extract_text(message: &AssistantMessage) -> String {
    let mut text = String::new();
    for block in &message.content {
        if let ContentBlock::Text { text: t } = block {
            text.push_str(t);
        }
    }
    text
}

/// Extract tool use descriptions from an assistant message
pub fn extract_tool_uses(message: &AssistantMessage) -> Vec<(String, String, Value)> {
    message
        .content
        .iter()
        .filter_map(|block| {
            if let ContentBlock::ToolUse { id, name, input } = block {
                Some((id.clone(), name.clone(), input.clone()))
            } else {
                None
            }
        })
        .collect()
}
