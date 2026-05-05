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
    name: String,
    arguments: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolResponse {
    #[serde(alias = "id")]
    call_id: String,
    #[serde(alias = "result")]
    response: String,
    #[serde(default)]
    use_model: bool,
    #[serde(default)]
    skip_ai: bool,
    #[serde(default)]
    timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WizardStatus {
    pending_calls: Vec<ToolCallRequest>,
    active_sessions: usize,
    recent_requests: Vec<IncomingRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IncomingRequest {
    id: String,
    model: String,
    user_message: String,
    tool_count: usize,
    timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelResponse {
    id: String,
    model: String,
    content: String,
    finish_reason: String,
    timestamp: String,
}

struct AppState {
    pending_calls: Mutex<Vec<ToolCallRequest>>,
    responses: Mutex<HashMap<String, ToolResponse>>,
    response_waiters: Mutex<HashMap<String, tokio::sync::oneshot::Sender<ToolResponse>>>,
    broadcast_tx: Sender<String>,
    recent_requests: Mutex<Vec<IncomingRequest>>,
    recent_responses: Mutex<Vec<ModelResponse>>,
}

impl AppState {
    fn new() -> Self {
        let (tx, _) = channel(100);
        Self {
            pending_calls: Mutex::new(Vec::new()),
            responses: Mutex::new(HashMap::new()),
            response_waiters: Mutex::new(HashMap::new()),
            broadcast_tx: tx,
            recent_requests: Mutex::new(Vec::new()),
            recent_responses: Mutex::new(Vec::new()),
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
        .btn-info { background: #17a2b8; color: white; }
        .btn-success { background: #007bff; color: white; }
        .btn:hover { opacity: 0.9; }
        textarea { width: 100%; min-height: 100px; padding: 10px; border: 1px solid #dee2e6; border-radius: 4px; font-family: monospace; }
        .log { background: #f8f9fa; border: 1px solid #dee2e6; padding: 15px; border-radius: 4px; max-height: 300px; overflow-y: auto; font-family: monospace; font-size: 12px; }
        .log-entry { margin-bottom: 5px; padding: 5px; background: white; border-radius: 3px; }
        .log-entry.info { border-left: 3px solid #17a2b8; }
        .log-entry.success { border-left: 3px solid #28a745; }
        .log-entry.error { border-left: 3px solid #dc3545; }
        .timestamp { color: #6c757d; font-size: 11px; }
        .response-card { background: #e7f3ff; padding: 15px; border-radius: 8px; margin-bottom: 15px; border-left: 4px solid #2196F3; }
        .response-card h4 { margin: 0 0 10px 0; color: #2196F3; }
        .response-card .wizard-input { background: #f0f0f0; padding: 10px; border-radius: 4px; margin-bottom: 10px; font-family: monospace; font-size: 12px; }
        .response-card .model-output { background: white; padding: 10px; border-radius: 4px; border: 1px solid #dee2e6; }
        .completed-tool { opacity: 0.7; pointer-events: none; }
        .request-card { background: #f0f8ff; padding: 12px; border-radius: 6px; margin-bottom: 10px; border-left: 3px solid #4a90e2; }
        .request-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
        .request-message { color: #333; font-size: 13px; margin-bottom: 6px; }
        .request-meta { color: #666; font-size: 11px; }
        .response-log-card { background: #fff3cd; padding: 12px; border-radius: 6px; margin-bottom: 10px; border-left: 3px solid #ffc107; }
        .response-log-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
        .response-log-reason { color: #856404; font-size: 11px; margin-bottom: 6px; font-weight: bold; }
        .response-log-content { color: #333; font-size: 12px; white-space: pre-wrap; word-break: break-word; }
        .delegation-card { background: #e7f3ff; padding: 12px; border-radius: 6px; margin-bottom: 10px; border-left: 3px solid #2196F3; }
        .delegation-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
        .delegation-args { color: #333; font-size: 11px; margin-bottom: 6px; font-family: monospace; white-space: pre-wrap; }
        .delegation-action { color: #1976d2; font-size: 11px; font-style: italic; }
        .tool-call-card { background: #fff3cd; padding: 12px; border-radius: 6px; margin-bottom: 10px; border-left: 3px solid #ffc107; }
        .tool-call-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
        .tool-call-status { color: #856404; font-size: 11px; margin-top: 8px; font-style: italic; }
    </style>
</head>
<body>
    <div class="header">
        <h1>🧙 Wizard of Oz - Tool Dispatch</h1>
        <div class="status">Connected • <span id="session-count">0</span> active sessions</div>
    </div>

    <div style="margin-top: 30px;">
        <h2>
            ⏱️ Event Stream
            <button onclick="toggleEventOrder()" style="margin-left: 15px; padding: 5px 10px; font-size: 12px; cursor: pointer;">
                Order: <span id="order-label">Newest first</span> ⬍
            </button>
        </h2>
        <div id="events" style="margin-top: 15px;"></div>
    </div>

    <div id="pending"></div>

    <div class="log" id="log"></div>

    <script>
        let ws;
        let currentCall = null;
        let newestFirst = true;  // Track event order

        function toggleEventOrder() {
            newestFirst = !newestFirst;
            document.getElementById('order-label').textContent = newestFirst ? 'Newest first' : 'Oldest first';
            reorderEvents();
            addLog(`Event order changed to: ${newestFirst ? 'newest first' : 'oldest first'}`, 'info');
        }

        function reorderEvents() {
            const eventsDiv = document.getElementById('events');
            const events = Array.from(eventsDiv.children);
            const sorted = events.sort((a, b) => {
                const timeA = new Date(a.querySelector('.timestamp')?.textContent || '0').getTime();
                const timeB = new Date(b.querySelector('.timestamp')?.textContent || '0').getTime();
                return newestFirst ? timeB - timeA : timeA - timeB;
            });
            sorted.forEach(card => eventsDiv.appendChild(card));
        }

        function addToEventStream(card) {
            const eventsDiv = document.getElementById('events');

            // Insert in timestamp order (oldest first)
            const cardTime = new Date(card.querySelector('.timestamp')?.textContent || 0).getTime();
            const children = Array.from(eventsDiv.children);

            let inserted = false;
            for (let i = 0; i < children.length; i++) {
                const childTime = new Date(children[i].querySelector('.timestamp')?.textContent || 0).getTime();
                if (cardTime < childTime) {
                    eventsDiv.insertBefore(card, children[i]);
                    inserted = true;
                    break;
                }
            }

            if (!inserted) {
                eventsDiv.appendChild(card);
            }

            // Reorder if needed
            if (newestFirst) {
                reorderEvents();
            }
        }

        function connect() {
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            ws = new WebSocket(`${protocol}//${window.location.host}/ws`);

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
                addPendingCall(data);
            } else if (data.type === 'response_complete') {
                addLog(`Response sent for ${data.call_id}: ${data.response.substring(0, 50)}...`, 'success');
                addModelResponse(data.call_id, data.wizard_response, data.model_response);
                clearPendingCall(data.call_id);
            } else if (data.type === 'log') {
                addLog(data.message, data.level || 'info');
            } else if (data.type === 'status') {
                updateStatus(data);
            } else if (data.type === 'incoming_request') {
                addIncomingRequest(data.data);
            } else if (data.type === 'model_response') {
                addModelResponseLog(data.data);
            } else if (data.type === 'delegation') {
                addDelegationEvent(data.data);
            } else if (data.type === 'tool_call_completed') {
                // Update the event stream card and clear from pending
                const eventCard = document.getElementById(`event-call-${data.tool_name}`);
                if (eventCard) {
                    const statusDiv = eventCard.querySelector('.tool-call-status');
                    if (statusDiv) {
                        statusDiv.textContent = '✓ Completed';
                        statusDiv.style.color = '#28a745';
                        statusDiv.style.fontWeight = 'bold';
                    }
                }

                // Also clear from pending section
                const pendingDiv = document.getElementById('pending');
                const toolCards = pendingDiv.querySelectorAll('.tool-call');
                toolCards.forEach(card => {
                    const title = card.querySelector('h3');
                    if (title && title.textContent.includes(data.tool_name)) {
                        clearPendingCall(card.id.replace('call-', ''));
                    }
                });
            }
        }

        function addModelResponse(callId, wizardInput, modelOutput) {
            const eventsDiv = document.getElementById('events');
            const card = document.createElement('div');
            card.className = 'response-card';
            card.id = `response-${callId}`;
            card.innerHTML = `
                <h4>✅ Tool Call Completed: ${callId.substring(0, 8)}...</h4>
                <div class="wizard-input">
                    <strong>Wizard provided:</strong><br>
                    ${wizardInput || '(empty)'}
                </div>
                <div class="model-output">
                    <strong>Model responded:</strong><br>
                    ${modelOutput || 'Waiting for model response...'}
                </div>
            `;
            eventsDiv.insertBefore(card, eventsDiv.firstChild);
        }

        function addPendingCall(call) {
            const timestamp = new Date().toLocaleTimeString();

            // Add to event stream (chronological)
            const card = document.createElement('div');
            card.className = 'tool-call-card';
            card.id = `event-call-${call.call_id}`;
            card.innerHTML = `
                <div class="tool-call-header">
                    <strong>🔧 Tool Call: ${call.name}</strong>
                    <span class="timestamp">${timestamp}</span>
                </div>
                <div><strong>ID:</strong> <code>${call.call_id}</code></div>
                <div><strong>Arguments:</strong></div>
                <pre>${JSON.stringify(call.arguments, null, 2)}</pre>
                <div class="tool-call-status">⏳ Waiting for wizard response...</div>
            `;
            addToEventStream(card);

            // Also add to pending section for interaction
            const pendingDiv = document.getElementById('pending');
            const callDiv = document.createElement('div');
            callDiv.className = 'tool-call';
            callDiv.id = `call-${call.call_id}`;
            callDiv.innerHTML = `
                <h3>🔧 Tool Call: ${call.name}</h3>
                <div><strong>ID:</strong> <code>${call.call_id}</code></div>
                <div><strong>Arguments:</strong></div>
                <pre>${JSON.stringify(call.arguments, null, 2)}</pre>
                <div class="actions">
                    <button class="btn btn-primary" onclick="respondDirect('${call.call_id}')">✍️ Answer as Tool</button>
                    <button class="btn btn-secondary" onclick="respondForward('${call.call_id}')">📝 Add Context & Forward to AI</button>
                    <button class="btn btn-info" onclick="respondSkip('${call.call_id}')">⚡ Skip AI (Testing)</button>
                    <button class="btn btn-warning" onclick="respondModel('${call.call_id}')">🤖 Let AI Answer</button>
                    <button class="btn btn-success" onclick="respondDelegate('${call.call_id}')">🔄 Delegate to Agent</button>
                </div>
                <div id="response-${call.call_id}" style="display:none; margin-top:15px;">
                    <label><strong>Your Response:</strong></label>
                    <textarea id="textarea-${call.call_id}" placeholder="Enter the tool result..."></textarea>
                    <div class="info" style="margin-top:10px; font-size:12px; color:#666;">
                        <strong>Mode:</strong> <span id="mode-${call.call_id}">answer_as_tool</span><br>
                        • <strong>Answer as Tool:</strong> Provide tool result, AI integrates into final answer<br>
                        • <strong>Add Context:</strong> Add info to conversation, AI continues<br>
                        • <strong>Skip AI:</strong> ⚠️ Currently doesn't work - AI still processes (engine limitation)<br>
                        • <strong>Let AI Answer:</strong> AI handles tool call itself<br>
                        • <strong>Delegate to Agent:</strong> Forward to original agent (Goose, etc.) to execute
                    </div>
                    <div class="actions" style="margin-top:10px;">
                        <button class="btn btn-primary" onclick="sendResponse('${call.call_id}')">Send Response</button>
                        <button class="btn btn-secondary" onclick="cancelResponse('${call.call_id}')">Cancel</button>
                        <button class="btn btn-success" onclick="respondDelegate('${call.call_id}')" style="display:none;" id="delegate-btn-${call.call_id}">Delegate to Agent</button>
                    </div>
                </div>
                <div id="submitted-${call.call_id}" style="display:none; margin-top:15px; background:#d4edda; padding:10px; border-radius:4px;">
                    <strong>✓ Submitted:</strong> <span id="submitted-text-${call.call_id}"></span>
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

        function addIncomingRequest(request) {
            const card = document.createElement('div');
            card.className = 'request-card';
            card.id = `req-${request.id}`;
            card.innerHTML = `
                <div class="request-header">
                    <strong>📨 Request: ${request.model}</strong>
                    <span class="timestamp">${new Date(request.timestamp).toLocaleTimeString()}</span>
                </div>
                <div class="request-message">${request.user_message.substring(0, 200)}${request.user_message.length > 200 ? '...' : ''}</div>
                <div class="request-meta">
                    Tools: ${request.tool_count || 0} | ID: ${request.id.substring(0, 8)}
                </div>
            `;
            addToEventStream(card);

            const eventsDiv = document.getElementById('events');
            // Keep only last 50 events visible
            while (eventsDiv.children.length > 50) {
                eventsDiv.removeChild(eventsDiv.lastChild);
            }

            addLog(`New request from ${request.model}: ${request.user_message.substring(0, 50)}...`, 'info');
        }

        function addModelResponseLog(response) {
            const card = document.createElement('div');
            card.className = 'response-log-card';
            card.id = `resp-${response.id}`;
            card.innerHTML = `
                <div class="response-log-header">
                    <strong>📤 Response: ${response.model}</strong>
                    <span class="timestamp">${new Date(response.timestamp).toLocaleTimeString()}</span>
                </div>
                <div class="response-log-reason">Finish: ${response.finish_reason}</div>
                <div class="response-log-content">${response.content.substring(0, 300)}${response.content.length > 300 ? '...' : ''}</div>
            `;
            addToEventStream(card);

            const eventsDiv = document.getElementById('events');
            // Keep only last 50 events visible
            while (eventsDiv.children.length > 50) {
                eventsDiv.removeChild(eventsDiv.lastChild);
            }

            const contentPreview = response.content.substring(0, 50);
            addLog(`Model response to ${response.model}: ${contentPreview}... (${response.finish_reason})`, 'success');
        }

        function addDelegationEvent(delegation) {
            const card = document.createElement('div');
            card.className = 'delegation-card';
            card.id = `delegation-${delegation.id}`;
            card.innerHTML = `
                <div class="delegation-header">
                    <strong>🔄 Delegated: ${delegation.tool_name}</strong>
                    <span class="timestamp">${new Date(delegation.timestamp).toLocaleTimeString()}</span>
                </div>
                <div class="delegation-args"><strong>Sent to agent:</strong><br>
${delegation.tool_args.substring(0, 300)}${delegation.tool_args.length > 300 ? '...' : ''}</div>
                <div class="delegation-action">→ Agent should execute tool and send back result</div>
            `;
            addToEventStream(card);

            const eventsDiv = document.getElementById('events');
            // Keep only last 50 events visible
            while (eventsDiv.children.length > 50) {
                eventsDiv.removeChild(eventsDiv.lastChild);
            }

            addLog(`Delegated ${delegation.tool_name} to agent (returned tool_calls in response)`, 'info');
        }


        function respondDirect(callId) {
            document.getElementById(`response-${callId}`).style.display = 'block';
            document.getElementById(`mode-${callId}`).textContent = 'answer_as_tool';
            document.getElementById(`textarea-${callId}`).placeholder = 'Enter tool result (e.g., {"temp": 20, "condition": "sunny"})...';
            document.getElementById(`textarea-${callId}`).focus();
        }

        function respondForward(callId) {
            document.getElementById(`response-${callId}`).style.display = 'block';
            document.getElementById(`mode-${callId}`).textContent = 'add_context_and_forward';
            document.getElementById(`textarea-${callId}`).placeholder = 'Enter context to add to conversation...';
            document.getElementById(`textarea-${callId}`).focus();
        }

        function respondSkip(callId) {
            document.getElementById(`response-${callId}`).style.display = 'block';
            document.getElementById(`mode-${callId}`).textContent = 'skip_ai';
            document.getElementById(`textarea-${callId}`).placeholder = 'Enter response to return directly (bypasses AI)...';
            document.getElementById(`textarea-${callId}`).focus();
        }

        function respondModel(callId) {
            sendResponse(callId, null, 'let_ai_answer');
        }

        function respondError(callId) {
            const errorResp = {
                name: "Error",
                message: "Tool execution failed (simulated error)",
                code: 500
            };
            sendResponse(callId, JSON.stringify(errorResp), false);
        }

        function respondDelegate(callId) {
            const textarea = document.getElementById(`textarea-${callId}`);
            const editedArgs = textarea.value.trim();
            sendResponse(callId, editedArgs || null, 'delegate');
        }


        function sendResponse(callId, customResponse, mode) {
            let result;
            let useModel = false;
            let skipAi = false;

            const finalMode = mode || document.getElementById(`mode-${callId}`).textContent;
            const textValue = document.getElementById(`textarea-${callId}`).value;

            if (finalMode === 'let_ai_answer') {
                result = JSON.stringify({ use_model: true, message: "Let model handle this" });
                useModel = true;
            } else if (finalMode === 'add_context_and_forward') {
                result = (textValue || customResponse) + "\n\n__FORWARD_TO_AI__";
            } else if (finalMode === 'skip_ai') {
                result = textValue || customResponse || JSON.stringify({ error: "No response provided" });
                skipAi = true;
            } else if (finalMode === 'delegate') {
                // Delegate to agent: optionally edit args, then forward
                const editedArgs = textValue || customResponse;
                if (editedArgs) {
                    result = `__DELEGATE__:${editedArgs}`;
                } else {
                    result = "__DELEGATE__";
                }
            } else if (textValue || customResponse) {
                result = textValue || customResponse;
            } else {
                result = JSON.stringify({ error: "No response provided" });
            }

            fetch('/respond', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    call_id: callId,
                    response: result,
                    use_model: useModel,
                    skip_ai: skipAi
                })
            }).then(r => r.json())
              .then(data => {
                  if (data.status === 'error') {
                      addLog(`Error: ${data.message}`, 'error');
                  } else {
                      // Show submitted response
                      document.getElementById(`response-${callId}`).style.display = 'none';
                      document.getElementById(`submitted-${callId}`).style.display = 'block';
                      document.getElementById(`submitted-text-${callId}`).textContent = result.substring(0, 100) + (result.length > 100 ? '...' : '');

                      addLog(`Response queued for ${callId} (${finalMode})`, 'success');
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
    let reqs = state.recent_requests.lock().unwrap();
    Json(WizardStatus {
        pending_calls: calls.clone(),
        active_sessions: 1,
        recent_requests: reqs.clone(),
    })
}

/// POST /respond - Wizard responds to a tool call
async fn respond_to_call(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ToolResponse>,
) -> impl IntoResponse {
    info!("WOZ: Wizard response received for call: {}", payload.call_id);
    let call_id = payload.call_id.clone();
    let timestamp = payload.timestamp.clone();

    // Check response mode
    let mode = if payload.skip_ai {
        "skip_ai"
    } else if payload.use_model {
        "let_ai_answer"
    } else if payload.response.contains("__FORWARD_TO_AI__") {
        "add_context_and_forward"
    } else {
        "answer_as_tool"
    };

    info!("WOZ: Response mode: {}", mode);

    // Add response to storage
    {
        let mut responses = state.responses.lock().unwrap();
        let response = ToolResponse {
            call_id: call_id.clone(),
            response: payload.response.clone(),
            use_model: payload.use_model,
            skip_ai: payload.skip_ai,
            timestamp: timestamp.clone(),
        };
        responses.insert(call_id.clone(), response);
    }

    // Notify waiter if exists
    {
        let mut waiters = state.response_waiters.lock().unwrap();
        if let Some(tx) = waiters.remove(&call_id) {
            info!("WOZ: Sending response to waiting dispatcher");
            let response = ToolResponse {
                call_id: call_id.clone(),
                response: payload.response.clone(),
                use_model: payload.use_model,
                skip_ai: payload.skip_ai,
                timestamp: timestamp.clone(),
            };
            let _ = tx.send(response);
        }
    }

    // Notify about response ready
    if let Ok(msg) = serde_json::to_string(&serde_json::json!({
        "type": "response_ready",
        "call_id": call_id,
        "response": payload.response,
        "mode": mode
    })) {
        info!("WOZ: Broadcasting wizard response");
        let _ = state.broadcast_tx.send(msg);
    } else {
        error!("WOZ: Failed to serialize response_ready message");
    }

    info!("WOZ: Wizard response queued for call {}", call_id);
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
/// Now blocks until wizard responds
async fn dispatch_tool(
    State(state): State<Arc<AppState>>,
    Json(mut tool_call): Json<ToolCallRequest>,
) -> impl IntoResponse {
    let call_id = uuid::Uuid::new_v4().to_string();
    info!("WOZ: Tool call received: {} (id: {})", tool_call.name, call_id);
    info!("WOZ: Arguments: {}", tool_call.arguments);

    // Update call_id
    tool_call.call_id = Some(call_id.clone());

    // Create channel for waiting for wizard response
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Add to pending calls and register waiter
    {
        let mut calls = state.pending_calls.lock().unwrap();
        calls.push(tool_call.clone());
        let mut waiters = state.response_waiters.lock().unwrap();
        waiters.insert(call_id.clone(), tx);
    }

    // Broadcast to wizard UI
    if let Ok(msg) = serde_json::to_string(&serde_json::json!({
        "type": "tool_call",
        "call_id": call_id,
        "name": tool_call.name,
        "arguments": tool_call.arguments
    })) {
        info!("WOZ: Broadcasting tool call to wizard UI");
        let _ = state.broadcast_tx.send(msg);
    } else {
        error!("WOZ: Failed to serialize broadcast message");
    }

    // Wait for wizard to respond (with timeout)
    info!("WOZ: Waiting for wizard response (30s timeout)");
    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(30),
        rx
    ).await;

    match response {
        Ok(Ok(wizard_response)) => {
            info!("WOZ: Received wizard response for call {}", call_id);

            // Clean up
            {
                let mut calls = state.pending_calls.lock().unwrap();
                calls.retain(|c| c.call_id.as_deref() != Some(&call_id));
                let mut waiters = state.response_waiters.lock().unwrap();
                waiters.remove(&call_id);
            }

            // Return content in format expected by tool dispatch
            let (content, complete) = if wizard_response.skip_ai {
                // Skip AI - return response directly (for testing)
                info!("WOZ: Wizard chose to skip AI for call {}", call_id);
                (wizard_response.response.clone(), true)
            } else if wizard_response.use_model {
                // Special case: let model handle it
                info!("WOZ: Wizard chose to let model handle call {}", call_id);
                ("{\"use_model\": true}".to_string(), false)
            } else if wizard_response.response.contains("__FORWARD_TO_AI__") {
                // Forward to AI with context
                info!("WOZ: Wizard chose to forward context to AI for call {}", call_id);
                let context = wizard_response.response.replace("__FORWARD_TO_AI__", "");
                (context, false)
            } else {
                // Answer as tool - let AI integrate the response
                info!("WOZ: Wizard answered as tool for call {}", call_id);
                (wizard_response.response.clone(), false)
            };

            let result = serde_json::json!({
                "content": content,
                "complete": complete
            });
            info!("WOZ: Returning response for call {} (complete: {})", call_id, complete);
            Json(result)
        }
        Ok(Err(e)) => {
            error!("WOZ: Channel error for call {}: {}", call_id, e);
            Json(serde_json::json!({
                "error": format!("Channel error: {}", e),
                "status": "failed"
            }))
        }
        Err(_) => {
            error!("WOZ: Timeout waiting for wizard response for call {}", call_id);
            // Clean up
            {
                let mut calls = state.pending_calls.lock().unwrap();
                calls.retain(|c| c.call_id.as_deref() != Some(&call_id));
                let mut waiters = state.response_waiters.lock().unwrap();
                waiters.remove(&call_id);
            }
            Json(serde_json::json!({
                "error": "Timeout waiting for wizard response",
                "status": "timeout"
            }))
        }
    }
}

/// GET /poll/{call_id} - Poll for tool response (called by mistral.rs)
async fn poll_response(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(call_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    info!("WOZ: Poll for response: {}", call_id);
    let responses = state.responses.lock().unwrap();

    // Check if response is ready
    if let Some(response) = responses.get(&call_id) {
        info!("WOZ: Response ready for call {}: {}", call_id,
            if response.use_model { "use_model" } else { "custom" });

        // Return the result in format expected by tool dispatch
        // Tool dispatch expects: {"content": "..."} or bare string
        let result = if response.use_model {
            // Special case: let model handle it
            "{\"use_model\": true}".to_string()
        } else {
            response.response.clone()
        };

        // Return as JSON with content field for tool dispatch compatibility
        let content_json = serde_json::json!({ "content": result });
        let content_str = content_json.to_string();

        // Clone response for broadcast
        let response_clone = response.clone();

        // Release lock before cleanup
        drop(responses);

        // Remove from pending
        {
            let mut calls = state.pending_calls.lock().unwrap();
            calls.retain(|c| c.call_id.as_deref() != Some(&call_id));
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
            "response": response_clone.response
        })) {
            let _ = state.broadcast_tx.send(msg);
        }

        // Return in format expected by mistral.rs tool dispatch
        info!("WOZ: Returning response for call {}", call_id);
        (axum::http::StatusCode::OK, content_str)
    } else {
        // Still waiting - return 202 with waiting status
        drop(responses);
        info!("WOZ: No response yet for call {}, still waiting", call_id);

        // Return error to signal waiting status to tool dispatch
        let error_json = serde_json::json!({
            "error": "Waiting for wizard response",
            "status": "pending"
        });
        (axum::http::StatusCode::ACCEPTED, error_json.to_string())
    }
}

/// POST /log_request - Log an incoming request from the main server
async fn log_request(
    State(state): State<Arc<AppState>>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = uuid::Uuid::new_v4().to_string();
    let model = request.get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let user_message = request.get("user_message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let tool_count = request.get("tool_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let timestamp = chrono::Utc::now().to_rfc3339();

    let incoming_request = IncomingRequest {
        id: id.clone(),
        model,
        user_message,
        tool_count,
        timestamp,
    };

    // Add to recent requests (keep last 50)
    {
        let mut reqs = state.recent_requests.lock().unwrap();
        reqs.push(incoming_request.clone());
        if reqs.len() > 50 {
            reqs.remove(0);
        }
    }

    // Broadcast to WebSocket clients
    if let Ok(msg) = serde_json::to_string(&serde_json::json!({
        "type": "incoming_request",
        "data": incoming_request
    })) {
        let _ = state.broadcast_tx.send(msg);
    }

    info!("WOZ: Logged incoming request {} (model: {}, tools: {})", id, request.get("model").and_then(|v| v.as_str()).unwrap_or("unknown"), tool_count);

    (axum::http::StatusCode::OK, Json(serde_json::json!({ "id": id })))
}

/// POST /log_response - Log a model response back to the client
async fn log_response(
    State(state): State<Arc<AppState>>,
    Json(response): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = uuid::Uuid::new_v4().to_string();
    let model = response.get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let content = response.get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let finish_reason = response.get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let timestamp = chrono::Utc::now().to_rfc3339();

    let model_response = ModelResponse {
        id: id.clone(),
        model,
        content,
        finish_reason,
        timestamp,
    };

    // Add to recent responses (keep last 50)
    {
        let mut resps = state.recent_responses.lock().unwrap();
        resps.push(model_response.clone());
        if resps.len() > 50 {
            resps.remove(0);
        }
    }

    // Broadcast to WebSocket clients
    if let Ok(msg) = serde_json::to_string(&serde_json::json!({
        "type": "model_response",
        "data": model_response
    })) {
        let _ = state.broadcast_tx.send(msg);
    }

    let content_preview = if model_response.content.len() > 100 {
        format!("{}...", &model_response.content[..100])
    } else {
        model_response.content.clone()
    };
    info!("WOZ: Logged model response {} (model: {}, reason: {}, content: {})",
        id, model_response.model, model_response.finish_reason, content_preview);

    (axum::http::StatusCode::OK, Json(serde_json::json!({ "id": id })))
}

/// POST /log_delegation - Log a tool delegation event
async fn log_delegation(
    State(state): State<Arc<AppState>>,
    Json(delegation): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = uuid::Uuid::new_v4().to_string();
    let tool_name = delegation.get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let tool_args = delegation.get("tool_args")
        .and_then(|v| v.as_str())
        .unwrap_or("{}")
        .to_string();

    let action = delegation.get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("delegated_to_client")
        .to_string();

    let timestamp = chrono::Utc::now().to_rfc3339();

    // Broadcast delegation event to WebSocket clients
    if let Ok(msg) = serde_json::to_string(&serde_json::json!({
        "type": "delegation",
        "data": {
            "id": id,
            "tool_name": tool_name.clone(),
            "tool_args": tool_args,
            "action": action,
            "timestamp": timestamp
        }
    })) {
        let _ = state.broadcast_tx.send(msg);
    }

    // Broadcast tool call completion to remove it from pending section
    if let Ok(msg) = serde_json::to_string(&serde_json::json!({
        "type": "tool_call_completed",
        "tool_name": tool_name,
        "timestamp": timestamp
    })) {
        let _ = state.broadcast_tx.send(msg);
    }

    info!("WOZ: Logged delegation {} - tool {} ({})", id, tool_name, action);

    (axum::http::StatusCode::OK, Json(serde_json::json!({ "id": id })))
}



/// Create router
pub fn create_wizard_router() -> Router {
    let state = Arc::new(AppState::new());
    info!("WOZ: Creating router with state");

    Router::new()
        .route("/", get(wizard_ui))
        .route("/status", get(get_status))
        .route("/dispatch", post(dispatch_tool))
        .route("/respond", post(respond_to_call))
        .route("/poll/{call_id}", get(poll_response))
        .route("/log_request", post(log_request))
        .route("/log_response", post(log_response))
        .route("/log_delegation", post(log_delegation))
        .route("/ws", get(wizard_websocket))
        .with_state(state)
}

