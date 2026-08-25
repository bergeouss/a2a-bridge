# A2A Bridge

A lightweight Rust server that exposes AI coding agents as [A2A](https://a2aprotocol.ai/) endpoints.

## Why?

Instead of driving a tmux session over SSH, A2A Bridge gives you a clean HTTP/JSON interface to any coding agent. Send tasks, get structured responses, manage sessions.

## Features

- **Multi-agent** — Claude Code, Codex, or any CLI agent via config
- **A2A v1 compatible** — Agent Card, JSON-RPC 2.0, SSE streaming
- **Session continuity** — Process stays alive between requests
- **On-demand** — Launch when needed, shutdown when done
- **Resume** — Restart with `--resume <session_id>` to continue a session
- **Lightweight** — ~2.7MB binary, ~10MB RAM while active

## Usage

### 1. Build

```bash
cargo build --release
# → target/release/a2a-bridge
```

### 2. Configure

```bash
cp agents.toml.example agents.toml
vim agents.toml
```

### 3. Run

```bash
# List available agents
./target/release/a2a-bridge --config agents.toml

# Start with selected agent
./target/release/a2a-bridge --config agents.toml --agent claude
```

### 4. Interact

```bash
# Health check
curl http://localhost:8742/health

# Get Agent Card
curl http://localhost:8742/.well-known/agent-card.json

# Send a message
curl -X POST http://localhost:8742/a2a \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-token" \
  -d '{"id":"1","method":"message/send","params":{"message":{"parts":[{"text":"Hello!"}]}}}'

# Get session ID
curl http://localhost:8742/session -H "Authorization: Bearer your-token"

# Shutdown
curl -X POST http://localhost:8742/shutdown -H "Authorization: Bearer your-token"
```

## Endpoints

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/health` | GET | ❌ | Health check |
| `/.well-known/agent-card.json` | GET | ❌ | A2A Agent Card |
| `/a2a` | POST | ✅ | JSON-RPC (message/send, agent/getCard) |
| `/a2a?message=...` | GET | ✅ | SSE streaming |
| `/session` | GET | ✅ | Get current session ID |
| `/shutdown` | POST | ✅ | Shutdown bridge gracefully |

## Session Management

- Sessions are maintained while the bridge is running
- Use `/session` to get the current session ID
- To resume a session: add `resume = "session-id"` in config and restart
- Sessions are saved by Claude Code when the bridge shuts down

## Configuration

### Server

| Key | Default | Description |
|-----|---------|-------------|
| `server.host` | `0.0.0.0` | Bind address |
| `server.port` | `8742` | Listen port |
| `server.auth_token` | `null` | Bearer token (null = no auth) |

### Agent definitions (`agents.*`)

| Key | Required | Description |
|-----|----------|-------------|
| `command` | ✅ | Executable path |
| `args` | ✅ | CLI arguments |
| `description` | ❌ | Shown in Agent Card |
| `workdir` | ❌ | Working directory |
| `timeout` | ❌ | Task timeout (default 600s) |
| `keep_alive` | ❌ | Keep process between requests (default true) |
| `resume` | ❌ | Session ID to resume |

### Adding a new agent

```toml
[agents.my-agent]
command = "/path/to/agent"
args = ["--stream-json"]
description = "My custom AI agent"
timeout = 300
```

## License

MIT
