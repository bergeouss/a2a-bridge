use axum::{
    extract::{Json, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::{Config, AgentDefinition};
use crate::lifeos_process::{LifeOsProcess, ProcessEvent};
use crate::sse;

/// Shared application state
pub struct AppState {
    pub config: Config,
    pub process: Arc<LifeOsProcess>,
    pub agent_name: String,
    pub agent_def: AgentDefinition,
}

/// JSON-RPC request structure
#[derive(Debug, serde::Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

/// JSON-RPC response structure
#[derive(Debug, serde::Serialize)]
struct JsonRpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, serde::Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

/// Build the router
pub fn create_router(state: Arc<Mutex<AppState>>) -> Router {
    Router::new()
        // Public routes (no auth)
        .route("/.well-known/agent-card.json", get(agent_card))
        .route("/health", get(health_check))
        // Protected routes (auth required)
        .route("/a2a", post(handle_a2a))
        .route("/a2a", get(handle_a2a_sse))
        .route("/session", get(get_session))
        .route("/shutdown", post(shutdown))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state)
}

/// Auth middleware — skips public routes
async fn auth_middleware(
    State(state): State<Arc<Mutex<AppState>>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Public routes — no auth needed
    if path == "/health" || path == "/.well-known/agent-card.json" {
        return next.run(request).await;
    }
    let token = state.lock().await.config.server.auth_token.clone();

    // If no auth token configured, allow all requests
    if token.is_none() {
        return next.run(request).await;
    }

    let expected = format!("Bearer {}", token.as_ref().unwrap());

    if let Some(auth_header) = request.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            // Constant-time comparison to prevent timing attacks
            if constant_time_eq::constant_time_eq(auth_str.as_bytes(), expected.as_bytes()) {
                return next.run(request).await;
            }
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": "Unauthorized",
            "message": "Missing or invalid Authorization header"
        })),
    )
        .into_response()
}

/// Agent Card endpoint
async fn agent_card(State(state): State<Arc<Mutex<AppState>>>) -> impl IntoResponse {
    let guard = state.lock().await;
    let base_url = format!(
        "http://{}:{}",
        guard.config.server.host, guard.config.server.port
    );
    let card = guard
        .config
        .agent_card(base_url, &guard.agent_name, &guard.agent_def);
    (StatusCode::OK, Json(card))
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

/// Main A2A handler (JSON-RPC over HTTP)
async fn handle_a2a(
    State(state): State<Arc<Mutex<AppState>>>,
    Json(request): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let method = request.method.clone();

    match method.as_str() {
        "message/send" => {
            let params = request.params.unwrap_or_default();
            let text = extract_message_text(&params);

            if text.is_empty() {
                return error_response(request.id, -32602, "Empty message".to_string());
            }

            // Clone process Arc to avoid holding state lock during send_message
            let process = state.lock().await.process.clone();
            let mut rx = match process.send_message(&text).await {
                Ok(rx) => rx,
                Err(e) => return error_response(request.id, -32603, e),
            };

            // Collect all events into a final response
            let mut text_parts: Vec<String> = Vec::new();
            let mut completed = None;

            while let Some(event) = rx.recv().await {
                match event {
                    ProcessEvent::TextDelta { text } => text_parts.push(text),
                    ProcessEvent::Completed { is_error, result_text, .. } => {
                        completed = Some((is_error, result_text));
                        break;
                    }
                    ProcessEvent::Error(msg) => {
                        return error_response(request.id, -32603, msg);
                    }
                    _ => {}
                }
            }

            let full_text = text_parts.join("");
            let result = if let Some((is_error, result_text)) = completed {
                if is_error {
                    serde_json::json!({
                        "state": "failed",
                        "error": result_text.unwrap_or_default()
                    })
                } else {
                    serde_json::json!({
                        "state": "completed",
                        "parts": [{ "type": "text", "text": full_text }]
                    })
                }
            } else {
                serde_json::json!({
                    "state": "completed",
                    "parts": [{ "type": "text", "text": full_text }]
                })
            };

            success_response(request.id, result)
        }
        "message/stream" => {
            // For SSE streaming, the GET handler should be used
            // But we can also support it here by returning a task ID
            let params = request.params.unwrap_or_default();
            let text = extract_message_text(&params);

            if text.is_empty() {
                return error_response(request.id, -32602, "Empty message".to_string());
            }

            let task_id = Uuid::new_v4().to_string();

            let guard = state.lock().await;
            let mut rx = match guard.process.send_message(&text).await {
                Ok(rx) => rx,
                Err(e) => return error_response(request.id, -32603, e),
            };
            drop(guard);

            // Spawn background task to process stream
            let state_clone = state.clone();
            tokio::spawn(async move {
                while let Some(_event) = rx.recv().await {
                    // Events would be stored in a task store for SSE retrieval
                    // For now, we just consume them
                }
            });

            success_response(
                request.id,
                serde_json::json!({
                    "task_id": task_id,
                    "state": "working",
                    "stream_url": format!("/a2a?task_id={}", task_id)
                }),
            )
        }
        "tasks/get" => {
            let params = request.params.unwrap_or_default();
            let _task_id = params.get("id").and_then(|v| v.as_str());

            success_response(
                request.id,
                serde_json::json!({
                    "state": "completed",
                    "message": "Task status retrieval not yet implemented for streaming tasks"
                }),
            )
        }
        "agent/getCard" => {
            let guard = state.lock().await;
            let base_url = format!(
                "http://{}:{}",
                guard.config.server.host, guard.config.server.port
            );
            success_response(
                request.id,
                guard
                    .config
                    .agent_card(base_url, &guard.agent_name, &guard.agent_def),
            )
        }
        _ => error_response(
            request.id,
            -32601,
            format!("Method '{}' not found", method),
        ),
    }
}

/// SSE streaming endpoint
async fn handle_a2a_sse(
    State(state): State<Arc<Mutex<AppState>>>,
    request: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let text = request.get("message").cloned().unwrap_or_default();

    if text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "Missing 'message' query parameter",
        ).into_response();
    }

    // Clone process Arc to avoid holding state lock during send_message
    let process = state.lock().await.process.clone();
    let rx = match process.send_message(&text).await {
        Ok(rx) => rx,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };

    sse::event_stream(rx).into_response()
}

/// Extract text from A2A message params
fn extract_message_text(params: &Value) -> String {
    params
        .get("message")
        .and_then(|m| m.get("parts"))
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .or_else(|| part.get("content"))
                        .and_then(|v| v.as_str())
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn success_response(id: Option<Value>, result: Value) -> (StatusCode, Json<JsonRpcResponse>) {
    (
        StatusCode::OK,
        Json(JsonRpcResponse {
            id,
            result: Some(result),
            error: None,
        }),
    )
}

fn error_response(id: Option<Value>, code: i32, message: String) -> (StatusCode, Json<JsonRpcResponse>) {
    (
        StatusCode::OK,
        Json(JsonRpcResponse {
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }),
    )
}

/// Get the current session ID
async fn get_session(State(state): State<Arc<Mutex<AppState>>>) -> impl IntoResponse {
    let guard = state.lock().await;
    let session_id = guard.process.get_session_id().await;
    match session_id {
        Some(id) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": id,
                "agent": guard.agent_name
            })),
        )
            .into_response(),
        None => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": null,
                "agent": guard.agent_name,
                "message": "No active session"
            })),
        )
            .into_response(),
    }
}

/// Shutdown the bridge gracefully
async fn shutdown(State(state): State<Arc<Mutex<AppState>>>) -> impl IntoResponse {
    let guard = state.lock().await;
    let session_id = guard.process.get_session_id().await;
    guard.process.kill().await;

    // Schedule server shutdown after response
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        std::process::exit(0);
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "shutting_down",
            "session_id": session_id
        })),
    )
}
