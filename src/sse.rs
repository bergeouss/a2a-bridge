use axum::response::{sse::Event, Sse};
use futures::stream::{self, Stream};
use std::convert::Infallible;

use crate::lifeos_process::ProcessEvent;

/// Convert a ProcessEvent stream into an SSE stream
pub fn event_stream(
    rx: tokio::sync::mpsc::Receiver<ProcessEvent>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(event) => {
                let sse_event = event_to_sse(&event);
                Some((Ok(sse_event), rx))
            }
            None => None,
        }
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn event_to_sse(event: &ProcessEvent) -> Event {
    match event {
        ProcessEvent::TextDelta { text } => Event::default()
            .event("artifact")
            .data(serde_json::json!({
                "parts": [{ "type": "text", "text": text }]
            }).to_string()),
        ProcessEvent::ToolUse { id, name, input } => Event::default()
            .event("status")
            .data(serde_json::json!({
                "state": "working",
                "message": format!("Using tool: {}", name),
                "tool": { "id": id, "name": name, "input": input }
            }).to_string()),
        ProcessEvent::ToolResult { tool_use_id, content, is_error } => Event::default()
            .event("status")
            .data(serde_json::json!({
                "state": "working",
                "message": format!("Tool result: {}", tool_use_id),
                "is_error": is_error
            }).to_string()),
        ProcessEvent::Completed { is_error, duration_ms, total_cost_usd, num_turns, result_text } => {
            let state = if *is_error { "failed" } else { "completed" };
            let mut data = serde_json::json!({
                "state": state
            });
            if let Some(dur) = duration_ms {
                data["duration_ms"] = serde_json::json!(dur);
            }
            if let Some(cost) = total_cost_usd {
                data["total_cost_usd"] = serde_json::json!(cost);
            }
            if let Some(turns) = num_turns {
                data["num_turns"] = serde_json::json!(turns);
            }
            if let Some(text) = result_text {
                data["result"] = serde_json::json!(text);
            }
            Event::default().event("status").data(data.to_string())
        }
        ProcessEvent::Error(msg) => Event::default()
            .event("status")
            .data(serde_json::json!({
                "state": "failed",
                "error": msg
            }).to_string()),
    }
}
