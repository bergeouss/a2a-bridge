use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::config::AgentDefinition;
use crate::ndjson;

/// Manages the long-lived LifeOS process
pub struct LifeOsProcess {
    config: AgentDefinition,
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    stdout: Arc<Mutex<Option<tokio::process::ChildStdout>>>,
    session_id: Arc<Mutex<Option<String>>>,
}

/// Events emitted by the process handler
#[derive(Debug, Clone)]
pub enum ProcessEvent {
    /// Text delta from assistant
    TextDelta { text: String },
    /// Tool call started
    ToolUse { id: String, name: String, input: serde_json::Value },
    /// Tool result received
    ToolResult { tool_use_id: String, content: serde_json::Value, is_error: bool },
    /// Turn completed
    Completed {
        is_error: bool,
        duration_ms: Option<u64>,
        total_cost_usd: Option<f64>,
        num_turns: Option<i64>,
        result_text: Option<String>,
    },
    /// Process error
    Error(String),
}

impl LifeOsProcess {
    pub fn new(config: AgentDefinition) -> Self {
        Self {
            config,
            child: Arc::new(Mutex::new(None)),
            stdin: Arc::new(Mutex::new(None)),
            stdout: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Ensure the process is running
    pub async fn ensure_running(&self) -> Result<(), String> {
        let mut child_guard = self.child.lock().await;

        if let Some(ref mut child) = *child_guard {
            match child.try_wait() {
                Ok(None) => return Ok(()), // Still running
                Ok(Some(status)) => {
                    tracing::warn!("LifeOS process exited with status: {}, restarting...", status);
                    *child_guard = None;
                }
                Err(e) => {
                    tracing::warn!("Failed to check process status: {}, restarting...", e);
                    *child_guard = None;
                }
            }
        }

        tracing::info!("Starting LifeOS process: {} {:?}", self.config.command, self.config.args);

        let mut cmd = Command::new(&self.config.command);
        let mut args = self.config.args.clone();
        if let Some(ref session_id) = self.config.resume {
            args.push("--resume".to_string());
            args.push(session_id.clone());
            tracing::info!("Resuming session: {}", session_id);
        }
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Add environment variables
        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        // Set working directory
        if let Some(ref workdir) = self.config.workdir {
            cmd.current_dir(workdir);
        }

        // Spawn the process
        let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn LifeOS process: {}", e))?;

        // Take ownership of stdin/stdout and store them
        let child_stdin = child.stdin.take().ok_or("Failed to open stdin")?;
        let child_stdout = child.stdout.take().ok_or("Failed to open stdout")?;

        tracing::info!("Stdin/Stdout taken from child process");
        *self.stdin.lock().await = Some(child_stdin);
        *self.stdout.lock().await = Some(child_stdout);
        *child_guard = Some(child);

        tracing::info!("LifeOS process started successfully");
        Ok(())
    }

    /// Send a message to LifeOS and stream the response
    pub async fn send_message(
        &self,
        message: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<ProcessEvent>, String> {
        // Ensure process is running
        self.ensure_running().await?;

        let (tx, rx) = tokio::sync::mpsc::channel(256);

        let stdin_arc = self.stdin.clone();
        let stdout_arc = self.stdout.clone();
        let child_arc = self.child.clone();
        let session_id_arc = self.session_id.clone();
        let timeout = self.config.timeout;
        let message = message.to_string();

        tokio::spawn(async move {
            let result = Self::handle_message(
                stdin_arc,
                stdout_arc,
                session_id_arc,
                &message,
                tx.clone(),
            )
            .await;

            if let Err(ref e) = result {
                let _ = tx.send(ProcessEvent::Error(e.clone())).await;
            }

            // Handle timeout
            if timeout > 0 {
                let child_arc = child_arc.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(timeout)).await;
                    let mut guard = child_arc.lock().await;
                    if let Some(ref mut child) = *guard {
                        let _ = child.kill().await;
                        tracing::warn!("LifeOS process killed after {}s timeout", timeout);
                    }
                });
            }
        });

        Ok(rx)
    }

    async fn handle_message(
        stdin_arc: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
        stdout_arc: Arc<Mutex<Option<tokio::process::ChildStdout>>>,
        session_id_arc: Arc<Mutex<Option<String>>>,
        message: &str,
        tx: tokio::sync::mpsc::Sender<ProcessEvent>,
    ) -> Result<(), String> {
        // Build the NDJSON input message
        let session_id = session_id_arc.lock().await.clone();
        let input = build_input_ndjson(message, session_id.as_deref());

        // Acquire stdin and write
        {
            let mut stdin_guard = stdin_arc.lock().await;
            match stdin_guard.as_mut() {
                Some(stdin) => {
                    tracing::info!("Writing to stdin: {}", &input[..input.len().min(50)]);
                    stdin
                        .write_all(input.as_bytes())
                        .await
                        .map_err(|e| format!("Failed to write to stdin: {}", e))?;
                    stdin
                        .write_all(b"\n")
                        .await
                        .map_err(|e| format!("Failed to write newline: {}", e))?;
                    stdin
                        .flush()
                        .await
                        .map_err(|e| format!("Failed to flush stdin: {}", e))?;
                    tracing::info!("Write to stdin OK");
                }
                None => {
                    tracing::error!("Stdin is None in handle_message!");
                    return Err("Stdin not available".to_string());
                }
            }
        }

        // Read stdout line by line
        let mut stdout_guard = stdout_arc.lock().await;
        let stdout = stdout_guard.as_mut().ok_or("Stdout not available")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    match ndjson::parse_line(trimmed) {
                        Ok(event) => {
                            // Update session_id if present
                            if let Some(ref sid) = get_session_id(&event) {
                                let mut sid_guard = session_id_arc.lock().await;
                                if sid_guard.is_none() {
                                    *sid_guard = Some(sid.clone());
                                }
                            }

                            // Convert to ProcessEvent and send
                            match event {
                                ndjson::StreamEvent::Assistant { message, .. } => {
                                    let text = ndjson::extract_text(&message);
                                    if !text.is_empty() {
                                        let _ = tx.send(ProcessEvent::TextDelta { text }).await;
                                    }
                                    for (id, name, input) in ndjson::extract_tool_uses(&message) {
                                        let _ = tx.send(ProcessEvent::ToolUse { id, name, input }).await;
                                    }
                                }
                                ndjson::StreamEvent::User { message, .. } => {
                                    for block in &message.content {
                                        if let ndjson::ContentBlock::ToolResult { tool_use_id, content, is_error } = block {
                                            let _ = tx.send(ProcessEvent::ToolResult {
                                                tool_use_id: tool_use_id.clone(),
                                                content: content.clone(),
                                                is_error: *is_error,
                                            }).await;
                                        }
                                    }
                                }
                                ndjson::StreamEvent::Result { subtype, is_error, duration_ms, total_cost_usd, num_turns, result_text, .. } => {
                                    let _ = tx.send(ProcessEvent::Completed {
                                        is_error: is_error || subtype != "success",
                                        duration_ms,
                                        total_cost_usd,
                                        num_turns,
                                        result_text,
                                    }).await;
                                    break; // End of turn
                                }
                                _ => {} // Ignore system, stream_event, unknown
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Failed to parse NDJSON line: {}", e);
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(ProcessEvent::Error(format!("Read error: {}", e))).await;
                    break;
                }
            }
        }

        Ok(())
    }

    /// Get the current session ID
    pub async fn get_session_id(&self) -> Option<String> {
        self.session_id.lock().await.clone()
    }

    /// Kill the process
    pub async fn kill(&self) {
        let mut guard = self.child.lock().await;
        if let Some(ref mut child) = *guard {
            let _ = child.kill().await;
            *guard = None;
            tracing::info!("LifeOS process killed");
        }
    }
}

/// Build NDJSON input message for Claude Code stream-json
fn build_input_ndjson(message: &str, session_id: Option<&str>) -> String {
    let mut json = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": message
        }
    });

    if let Some(sid) = session_id {
        json["session_id"] = serde_json::json!(sid);
    }

    json.to_string()
}

/// Extract session_id from a stream event
fn get_session_id(event: &ndjson::StreamEvent) -> Option<String> {
    match event {
        ndjson::StreamEvent::System { session_id, .. } => session_id.clone(),
        ndjson::StreamEvent::Assistant { session_id, .. } => session_id.clone(),
        ndjson::StreamEvent::User { session_id, .. } => session_id.clone(),
        ndjson::StreamEvent::Result { session_id, .. } => session_id.clone(),
        ndjson::StreamEvent::StreamEvent { session_id, .. } => session_id.clone(),
        _ => None,
    }
}
