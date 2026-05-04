//! Wizard of Oz Tool Dispatch Server
//!
//! HTTP server that intercepts tool calls and presents them to a human wizard
//! who can either respond themselves or let the model auto-generate.
//!
//! Features:
//! - Web UI for viewing pending tool calls
//! - Wizard can provide custom responses or let model answer
//! - Completely transparent to end user
//! - Tests both tool calling format AND client response handling

use axum::{
    extract::{State, WebSocketUpgrade},
    response::{IntoResponse, Json, Html},
    routing::{get, post},
    Router,
};
use axum::extract::ws::Message;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast::{channel, Sender};
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallRequest {
    id: String,
    function: ToolCallFunction,
    #[serde(default)]
    context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallFunction {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolResponse {
    id: String,
    result: String,
    #[serde(default)]
    use_model: bool,
    #[serde(default)]
    timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WizardStatus {
    pending_calls: Vec<ToolCallRequest>,
    active_sessions: usize,
}

struct AppState {
    pending_calls: Mutex<Vec<ToolCallRequest>>,
    responses: Mutex<HashMap<String, ToolResponse>>,
    broadcast_tx: Sender<String>,
}

impl AppState {
    fn new() -> Self {
        let (tx, _) = channel(100);
        Self {
            pending_calls: Mutex::new(Vec::new()),
            responses: Mutex::new(HashMap::new()),
            broadcast_tx: tx,
        }
    }
}

/// GET / - Web UI for wizard
async fn wizard_ui() -> impl IntoResponse {
    let html = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Wizard of Oz - Tool Dispatch</title>
    <style>
        body { font-family: system-ui; sans-serif; max-width: 1200px; margin: 40px auto; padding: 0 20px; }
        .header { background: #2c3e50; color: white; padding: 20px; border-radius: 8px; margin-bottom: 20px; }
        .status { font-size: 14px; opacity: 0.9; margin-top: 10px; }
        .pending { background: #fff3cd; padding: 15px; border-left: 4px solid #ffc107; border-radius: 4px; margin: 20px 0; }
        .tool-call { background: #f8f9fa; border: 1px solid #dee2e6; padding: 15px; border-radius: 8px; margin-bottom: 15px; }
        .tool-call h3 { margin: 0 0 10px 0; color: #2c3e50; }
        .tool-call pre { background: #f1f3f5; padding: 10px; border-radius: 4px; overflow-x: auto; }
        .actions { margin-top: 15px; }
        .btn { padding: 10px 20px; margin-right: 10px; border: none; border-radius: 4px; cursor: pointer; font-size: 14px; }
        .btn-primary { background: #28a745; color: white; }
        .btn-secondary { background: #6c757d; color: white; }
        .btn-warning { background: #ffc107; color: #212529; }
        .btn:hover { opacity: 0.9; }
        textarea { width: 100%; min-height: 100px; padding: 10px; border: 1px solid #dee2e6; border-radius: 4px; font-family: monospace; }
        .log { background: #f8f9fa; border: 1px solid #dee2e6; padding: 15px; border-radius: 4px; max-height: 300px; overflow-y: auto; font-family: monospace; font-size: 12px; }
        .log-entry { margin-bottom: 5px; padding: 5px; background: white; border-radius: 3px; }
        .log-entry.info { border-left: 3px solid #17a2b8; }
        .log-entry.success { border-left: 3px solid #28a745; }
        .log-entry.error { border-left: 3px solid #dc3545; }
        .timestamp { color: #6c757d; font-size: 11px; }
    </style>
</head>
<body>
    <div class="header">
        <h1>🧙 Wizard of Oz - Tool Dispatch</h1>
        <div class="status">Connected • <span id="session-count">0</span> active sessions</div>
    </div>

    <div id="pending"></div>

    <div class="log" id="log"></div>

    <script>
        let ws;
        let currentCall = null;

        function connect() {
            ws = new WebSocket('ws://localhost:7890/ws');

            ws.onopen = () => addLog('Connected to Wizard server', 'info');
            ws.onmessage = (event) => {
                const data = JSON.parse(event.data);
                handleServerEvent(data);
            };
            ws.onclose = () => {
                addLog('Disconnected. Reconnecting in 3s...', 'error');
                setTimeout(connect, 3000);
            };
            ws.onerror = () => addLog('WebSocket error', 'error');
        }

        function handleServerEvent(data) {
            if (data.type === 'tool_call') {
                addPendingCall(data.call);
            } else if (data.type === 'response_complete') {
                addLog(`Response sent for ${data.call_id}: ${data.response.substring(0, 50)}...`, 'success');
                clearPendingCall(data.call_id);
            } else if (data.type === 'log') {
                addLog(data.message, data.level || 'info');
            } else if (data.type === 'status') {
                updateStatus(data);
            }
        }

        function addPendingCall(call) {
            currentCall = call;
            const pendingDiv = document.getElementById('pending');
            const callDiv = document.createElement('div');
            callDiv.className = 'tool-call';
            callDiv.id = `call-${call.id}`;
            callDiv.innerHTML = `
                <h3>🔧 Tool Call: ${call.function.name}</h3>
                <div><strong>ID:</strong> <code>${call.id}</code></div>
                <div><strong>Arguments:</strong></div>
                <pre>${JSON.stringify(call.function.arguments, null, 2)}</pre>
                ${call.context ? `<div><strong>Context:</strong> ${call.context}</div>` : ''}
                <div class="actions">
                    <button class="btn btn-primary" onclick="respondCustom('${call.id}')">✍️ Custom Response</button>
                    <button class="btn btn-secondary" onclick="respondModel('${call.id}')">🤖 Let Model Respond</button>
                    <button class="btn btn-warning" onclick="respondError('${call.id}')">❌ Simulate Error</button>
                </div>
                <div id="response-${call.id}" style="display:none; margin-top:15px;">
                    <label><strong>Your Response:</strong></label>
                    <textarea id="textarea-${call.id}" placeholder="Enter the tool result..."></textarea>
                    <div class="actions">
                        <button class="btn btn-primary" onclick="sendResponse('${call.id}')">Send Response</button>
                        <button class="btn btn-secondary" onclick="cancelResponse('${call.id}')">Cancel</button>
                    </div>
                </div>
            `;
            pendingDiv.appendChild(callDiv);
        }

        function clearPendingCall(callId) {
            const callDiv = document.getElementById(`call-${callId}`);
            if (callDiv) {
                callDiv.style.opacity = '0.5';
                callDiv.style.pointerEvents = 'none';
                setTimeout(() => callDiv.remove(), 2000);
            }
        }

        function respondCustom(callId) {
            document.getElementById(`response-${callId}`).style.display = 'block';
            document.getElementById(`textarea-${callId}`).focus();
        }

        function respondModel(callId) {
            sendResponse(callId, null, true);
        }

        function respondError(callId) {
            const errorResp = {
                name: "Error",
                message: "Tool execution failed (simulated error)",
                code: 500
            };
            sendResponse(callId, JSON.stringify(errorResp), false);
        }

        function sendResponse(callId, customResponse, useModel) {
            let result;
            if (useModel) {
                result = JSON.stringify({ use_model: true, message: "Let model handle this" });
            } else if (customResponse) {
                result = document.getElementById(`textarea-${callId}`).value;
            } else {
                result = JSON.stringify({ error: "No response provided" });
            }

            fetch('/respond', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    call_id: callId,
                    response: result,
                    use_model: useModel || false
                })
            }).then(r => r.json())
              .then(data => {
                  if (data.status === 'error') {
                      addLog(`Error: ${data.message}`, 'error');
                  } else {
                      addLog(`Response queued for ${callId}`, 'success');
                      clearPendingCall(callId);
                  }
              });
        }

        function cancelResponse(callId) {
            document.getElementById(`response-${callId}`).style.display = 'none';
        }

        function addLog(message, level = 'info') {
            const logDiv = document.getElementById('log');
            const entry = document.createElement('div');
            entry.className = `log-entry ${level}`;
            entry.innerHTML = `<span class="timestamp">${new Date().toLocaleTimeString()}</span> ${message}`;
            logDiv.insertBefore(entry, logDiv.firstChild);
        }

        function updateStatus(data) {
            document.getElementById('session-count').textContent = data.active_sessions;
        }

        connect();
    </script>
</body>
</html>
    "#;

    Html(html)
}

/// GET /status - Current status for wizards
async fn get_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let calls = state.pending_calls.lock().unwrap();
    Json(WizardStatus {
        pending_calls: calls.clone(),
        active_sessions: 1,
    })
}

/// POST /respond - Wizard responds to a tool call
async fn respond_to_call(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ToolResponse>,
) -> impl IntoResponse {
    let mut responses = state.responses.lock().unwrap();
    let call_id = payload.id.clone();

    // Add response
    responses.insert(call_id.clone(), payload.clone());

    // Notify about response ready
    if let Ok(msg) = serde_json::to_string(&serde_json::json!({
        "type": "response_ready",
        "call_id": call_id,
        "response": payload
    })) {
        let _ = state.broadcast_tx.send(msg);
    }

    Json(serde_json::json!({
        "status": "queued",
        "call_id": call_id
    }))
}

/// WebSocket endpoint for real-time updates
async fn wizard_websocket(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        let rx = state.broadcast_tx.subscribe();

        // Send connection confirmation
        if let Ok(msg) = serde_json::to_string(&serde_json::json!({
            "type": "connected",
            "message": "Connected to Wizard of Oz dispatch server"
        })) {
            let _ = socket.send(Message::Text(msg.into())).await;
        }

        // Handle incoming messages from wizard
        let mut rx2 = rx.resubscribe();
        loop {
            tokio::select! {
                result = socket.recv() => {
                    match result {
                        Some(Ok(msg)) => {
                            if let Message::Text(text) = msg {
                                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                                    if let Some(msg_type) = data.get("type").and_then(|v| v.as_str()) {
                                        if msg_type == "ping" {
                                            if let Ok(pong) = serde_json::to_string(&serde_json::json!({"type": "pong"})) {
                                                let _ = socket.send(Message::Text(pong.into())).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            error!("WebSocket error: {}", e);
                            break;
                        }
                        None => break,
                    }
                }
                result = rx2.recv() => {
                    match result {
                        Ok(msg) => {
                            let _ = socket.send(Message::Text(msg.into())).await;
                        }
                        Err(e) => {
                            error!("Broadcast error: {}", e);
                            break;
                        }
                    }
                }
            }
        }
    })
}

/// POST /dispatch - Called by mistral.rs when tool is dispatched
async fn dispatch_tool(
    State(state): State<Arc<AppState>>,
    Json(tool_call): Json<ToolCallRequest>,
) -> impl IntoResponse {
    info!("Tool call received: {} (id: {})", tool_call.function.name, tool_call.id);

    // Add to pending calls
    {
        let mut calls = state.pending_calls.lock().unwrap();
        calls.push(tool_call.clone());
    }

    // Broadcast to wizard UI
    if let Ok(msg) = serde_json::to_string(&serde_json::json!({
        "type": "tool_call",
        "call": tool_call
    })) {
        let _ = state.broadcast_tx.send(msg);
    }

    // Wait for wizard to respond (polling endpoint)
    // In real implementation, this would use async channel
    Json(serde_json::json!({
        "status": "pending",
        "call_id": tool_call.id,
        "message": "Tool call queued. Wizard will respond via /respond endpoint.",
        "poll_url": format!("http://localhost:7890/poll/{}", tool_call.id)
    }))
}

/// GET /poll/{call_id} - Poll for tool response (called by mistral.rs)
async fn poll_response(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(call_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let responses = state.responses.lock().unwrap();

    // Check if response is ready
    if let Some(response) = responses.get(&call_id) {
        info!("Returning response for call {}", call_id);

        // Return the result
        let result = if response.use_model {
            // Special case: let model handle it
            "{\"use_model\": true}".to_string()
        } else {
            response.result.clone()
        };

        // Clone response for broadcast
        let response_clone = response.clone();

        // Release lock before cleanup
        drop(responses);

        // Remove from pending
        {
            let mut calls = state.pending_calls.lock().unwrap();
            calls.retain(|c| c.id != call_id);
        }

        // Clean up response
        {
            let mut responses = state.responses.lock().unwrap();
            responses.remove(&call_id);
        }

        // Broadcast completion
        if let Ok(msg) = serde_json::to_string(&serde_json::json!({
            "type": "response_complete",
            "call_id": call_id,
            "response": response_clone.result
        })) {
            let _ = state.broadcast_tx.send(msg);
        }

        // Return in format expected by mistral.rs tool dispatch
        (axum::http::StatusCode::OK, result)
    } else {
        // Still waiting
        drop(responses);
        let waiting = serde_json::json!({
            "status": "waiting",
            "message": "Waiting for wizard response"
        });
        (axum::http::StatusCode::ACCEPTED, waiting.to_string())
    }
}

/// Create router
pub fn create_wizard_router() -> Router {
    let state = Arc::new(AppState::new());

    Router::new()
        .route("/", get(wizard_ui))
        .route("/status", get(get_status))
        .route("/dispatch", post(dispatch_tool))
        .route("/respond", post(respond_to_call))
        .route("/poll/{call_id}", get(poll_response))
        .route("/ws", get(wizard_websocket))
        .with_state(state)
}

