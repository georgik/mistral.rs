# Wizard of Oz Tool Testing

The Wizard of Oz (WOZ) server provides debugging interfaces for testing and developing agentic applications with tool calling. It allows you to intercept, inspect, and manually respond to tool calls, making it easier to test tool workflows during development.

## Overview

When developing agentic applications, you need to verify that:
- The model correctly identifies when to call tools
- Tool parameters are properly structured
- Your agent handles tool responses correctly
- Tool execution flows work as expected

WOZ provides two modes:
1. **Interactive server mode** (new): Real-time web UI for manual tool interception
2. **File-based mode** (legacy): Pre-defined responses from JSONL files

Both modes act as a man-in-the-middle for tool calls, giving you full visibility and control over the tool execution loop.

## Interactive Server Mode

The interactive WOZ server provides a real-time web interface for monitoring and controlling tool execution.

### Quick Start

```bash
# Start main server with WOZ enabled on port 7890
mistralrs serve --wizard-mode auto --wizard-port 7890 -m Qwen/Qwen3-4B
```

This starts:
- Main inference server on port 8080 (or configured port)
- WOZ debugging interface on port 7890

### Access the Web UI

Open your browser to `http://localhost:7890`

The UI shows:
- Live event stream with all requests, responses, and tool calls
- Pending tool calls waiting for your response
- Controls for how to handle each tool call
- Chronological event ordering

### Tool Response Modes

When the model calls a tool, you can choose from three response modes:

#### 1. Answer as Tool

Provide a simulated tool result. The tool loop continues and the model sees your response.

**Use when**: You want to test how the model handles different tool outputs without implementing the actual tool.

**Example**: Model calls `tree` tool to list files. You respond with:
```
src/
  main.rs
  Cargo.toml
```

#### 2. Delegate to Agent

Forward the tool call back to your agent (Goose, LangChain, etc.) for actual execution.

**Use when**: Your agent implements the tool and you want to test the full execution flow.

**How it works**:
- WOZ returns the tool call to the agent with `finish_reason: "tool_calls"`
- Agent executes the tool locally
- Agent sends back a tool response message
- Model processes the result and continues

#### 3. Skip AI (Complete)

Return a final answer and stop the tool loop immediately.

**Use when**: You want to test how the agent handles premature completion or provide a canned response.

### Workflow Example

1. **Start servers**:
   ```bash
   mistralrs serve --wizard-mode auto --wizard-port 7890 -m Qwen/Qwen3-4B
   ```

2. **Send request from your agent**:
   ```python
   response = client.chat.completions.create(
       model="Qwen3-4B",
       messages=[{"role": "user", "content": "List files in /tmp"}],
       tools=[{
           "type": "function",
           "function": {
               "name": "tree",
               "description": "List directory tree",
               "parameters": {
                   "type": "object",
                   "properties": {
                       "path": {"type": "string"}
                   },
                   "required": ["path"]
               }
           }
       }]
   )
   ```

3. **Monitor in WOZ UI**:
   - Request appears in event stream
   - Model generates text, then calls `tree` tool
   - Tool call appears with parameters

4. **Choose response mode**:
   - Click "Delegate to Agent" to let your agent execute it
   - Or provide a custom response to test the model's behavior

5. **Observe the flow**:
   - Tool execution appears in event stream
   - Model processes tool result
   - Loop continues if more tools needed
   - Final response returned to agent

### Architecture

```
┌─────────┐         ┌──────────────┐         ┌──────────┐
│ Agent   │ ──────> │ mistral.rs   │ ──────> │  Model   │
│ (Goose) │ <─────  │   Server     │ <─────  │          │
└─────────┘         └──────────────┘         └──────────┘
                            │
                            │ tool_calls
                            ▼
                     ┌──────────────┐
                     │ WOZ Server   │
                     │  (port 7890) │
                     └──────────────┘
                            │
                ┌───────────┼───────────┐
                ▼           ▼           ▼
           ┌─────────┐ ┌────────┐ ┌─────────┐
           │ Delegate│ │ Answer │ │ Complete│
           │ to Agent│ │ as Tool│ │   (AI)  │
           └─────────┘ └────────┘ └─────────┘
```

### API Endpoints

The WOZ server exposes several endpoints for logging and debugging:

#### POST /log_request
Logged when a new request arrives.

**Request body**:
```json
{
  "model": "Qwen3-4B",
  "user_message": "List files in /tmp",
  "tool_count": 5
}
```

#### POST /log_response
Logged when a response completes.

**Request body**:
```json
{
  "model": "Qwen3-4B",
  "finish_reason": "tool_calls",
  "content": "...",
  "tool_calls": [...]
}
```

#### POST /dispatch
Main tool dispatch endpoint. Called by mistral.rs when a tool is invoked.

**Request body**:
```json
{
  "name": "tree",
  "arguments": {"path": "/tmp"}
}
```

**Response formats**:

Normal tool execution:
```json
{
  "content": "src/\n  main.rs\n"
}
```

Delegation (send back to agent):
```
__DELEGATE__
```

Complete (stop tool loop):
```json
{
  "content": "Task completed successfully",
  "complete": true
}
```

#### POST /log_delegation
Logged when a tool is delegated to the agent.

#### WebSocket /ws
Real-time event stream for the UI.

### Debugging with Interactive Mode

**Agent ends conversation without executing tools**

Symptom: Tool call appears in WOZ UI, agent stops responding

Cause: Tool call chunks not being forwarded to agent

Solution: Check server logs for "Delegation detected" message. Verify streaming chunks include tool_calls.

**Model hallucinates tool results**

Symptom: Model generates text response instead of calling tool, or invents tool output

Cause: Tool loop feeding tool calls back into model instead of delegating

Solution: Verify delegation code returns response to client immediately, not continuing tool loop.

**Multiple tool calls in sequence**

Symptom: Model calls tool, gets response, immediately calls another tool

Cause: Model expects multi-step tool execution

Solution: Use Delegate mode to let agent handle execution flow, or provide progressively detailed responses.

## File-Based Wizard Mode (Legacy)

## How It Works

1. Model generates a tool call (or tries to)
2. Tool call is intercepted and saved to `wizard_tool_calls.jsonl`
3. Your pre-defined response is loaded from `wizard_responses.jsonl`
4. Response is returned to the model
5. Model processes response and continues conversation

## Usage

### 1. Start Server with Wizard Mode

```bash
export MISTRALRS_WIZARD_DIR=$(pwd)
cargo run --release --bin mistralrs-server -- --tool-dispatch wizard
```

### 2. Set Environment Variable

```bash
export MISTRALRS_WIZARD_DIR=/path/to/your/workspace
```

The wizard mode creates two files in this directory:
- `wizard_tool_calls.jsonl` - Log of all tool calls made
- `wizard_responses.jsonl` - Your pre-defined responses

### 3. Create Pre-defined Responses

Edit `wizard_responses.jsonl`:

```jsonl
{"id": "call_123", "result": "File hello.py created successfully with content: print('hello world')"}
{"id": "call_456", "result": "Weather in Boston: 22.5°C, sunny, humidity 45%"}
```

Each response needs:
- `id`: The tool call ID (matches what's logged in `wizard_tool_calls.jsonl`)
- `result`: The string result to return to the model

### 4. Use with Goose

```bash
# In one terminal:
export MISTRALRS_WIZARD_DIR=$(pwd)
cargo run --release --bin mistralrs-server -- --tool-dispatch wizard

# In another terminal:
cargo run --release --bin goose
```

When goose triggers a tool call:
1. Check `wizard_tool_calls.jsonl` for the call details
2. Add your response to `wizard_responses.jsonl`
3. Restart goose or wait for next tool call
4. Model receives your response and continues

## Example: Testing Write Tool

### Step 1: Check the Tool Call

`wizard_tool_calls.jsonl`:
```json
{"id":"call_abc","function":"write","arguments":{"path":"hello.py","content":"print('hello')"},"timestamp":"2026-05-04T17:00:00Z"}
```

### Step 2: Add Your Response

`wizard_responses.jsonl`:
```json
{"id":"call_abc","result":"File hello.py created successfully."}
```

### Step 3: Model Continues

Model receives: "File hello.py created successfully."

Model then generates its final response incorporating this result.

## Benefits

- **Test model understanding**: Verify model correctly processes tool results
- **Debug tool format**: Test different response formats without re-running tools
- **No tool execution**: Safe testing without side effects
- **Reproducible tests**: Use same responses for regression testing
- **Interactive debugging**: Modify responses in real-time to test scenarios

## Tool Call ID Matching

The `id` field in responses must match the `id` from tool calls exactly. IDs are auto-generated UUIDs, so check `wizard_tool_calls.jsonl` first.

## Troubleshooting

**No response found**:
- Check that `id` in your response matches the tool call ID in `wizard_tool_calls.jsonl`
- Verify `MISTRALRS_WIZARD_DIR` is set correctly
- Check server logs for "Wizard of Oz" messages

**Tool calls not being intercepted**:
- Verify server started with `--tool-dispatch wizard`
- Check that `tool_dispatch_url` is being set (check server logs)

**Empty responses**:
- Ensure `result` field is present and contains a string
- Check for JSON syntax errors in `wizard_responses.jsonl`

## Configuration

### CLI Flags

Interactive server mode:
- `--wizard-mode auto`: Enable WOZ server alongside main server
- `--wizard-port 7890`: Port for WOZ server (default: 7890)

File-based mode:
- `--tool-dispatch wizard`: Use file-based wizard mode
- `--tool-dispatch http://URL`: POST tool calls to external HTTP endpoint

### Environment Variables

- `MISTRALRS_WIZARD_DIR`: Directory for wizard_responses.jsonl (file-based mode only)
- `MISTRALRS_TOOL_DISPATCH_URL`: Override tool dispatch URL

### Integration with Agents

#### Goose

Goose works seamlessly with WOZ delegation:

1. Goose sends request with tools
2. Model generates tool call
3. WOZ intercepts and delegates
4. Goose receives tool_calls with `finish_reason: "tool_calls"`
5. Goose executes tool locally
6. Goose sends tool result message back
7. Model continues

#### Other Agents

Any OpenAI-compatible agent can use WOZ:
- LangChain
- AutoGen
- OpenAI SDKs
- Custom agents

Just point the agent at the mistral.rs server with `--wizard-mode` enabled.

## Performance and Security

### Performance

- WOZ server adds minimal overhead (~1-2ms per tool call)
- WebSocket connection efficient for real-time updates
- No impact on inference performance
- Can be disabled in production by removing `--wizard-mode` flag

### Security

**Important**: WOZ server binds to `0.0.0.0:7890` by default, making it accessible from your network.

Recommendations for production:
- Use firewall rules to restrict access (allow only localhost)
- Consider using reverse proxy (nginx, traefik) for authentication
- Never expose WOZ interface publicly in production
- Disable wizard mode in production deployments

Example firewall rule:
```bash
# Allow only localhost access to WOZ server
iptables -A INPUT -p tcp --dport 7890 -s 127.0.0.1 -j ACCEPT
iptables -A INPUT -p tcp --dport 7890 -j DROP
```

## Logging

The server logs all activities at INFO level:

```bash
# Request logging
INFO mistralrs_server_core::chat_completion: Logging request to WOZ: http://localhost:7890/log_request

# Tool call received
INFO mistralrs_server_core::wizard_ofoz: WOZ: Tool call received: tree (id: ...)

# Delegation detected
INFO mistralrs_core::engine::search_request: Delegation detected, sending tool_calls response to client

# Response completion
INFO mistralrs_server_core::chat_completion: Logging streaming response to WOZ: (model: Qwen3-4B, reason: stop, content: ...)
```

Enable debug logging for more details:
```bash
RUST_LOG=debug mistralrs serve --wizard-mode auto -m Qwen/Qwen3-4B
```

## Advanced Usage

### Custom Dispatch Endpoint

Forward tools to your own HTTP service:

```bash
mistralrs serve --tool-dispatch-url https://my-service.com/tools -m Qwen/Qwen3-4B
```

Your service receives tool calls and returns responses in the expected format.

### Combining with Web Search

WOZ works with mistral.rs web search integration:

```bash
mistralrs serve --wizard-mode auto --enable-search -m Qwen/Qwen3-4B
```

You can then intercept and manually respond to search/extract tools for testing.

### Multiple Tool Rounds

Monitor complex multi-step tool workflows:
1. Model calls tool A
2. You provide response
3. Model calls tool B based on result
4. You provide response
5. Model synthesizes final answer

The WOZ UI shows the complete chain with timestamps for debugging.

