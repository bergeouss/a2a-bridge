# A2A Bridge Skill

Interact with Claude Code via A2A protocol.

## Setup

1. Build: `cargo build --release`
2. Deploy binary to target machine
3. Create config TOML
4. Run: `a2a-bridge --config agents.toml`

## Lifecycle

```
Launch → Message(s) → Shutdown
                ↑
         (optional: resume later)
```

## Quick Start

### Launch
```bash
a2a-bridge --config agents.toml
```

### Send message
```bash
curl -X POST http://HOST:PORT/a2a \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer TOKEN" \
  -d '{"id":"1","method":"message/send","params":{"message":{"parts":[{"text":"Hello"}]}}}'
```

### Get session ID
```bash
curl http://HOST:PORT/session -H "Authorization: Bearer TOKEN"
```

### Shutdown
```bash
curl -X POST http://HOST:PORT/shutdown -H "Authorization: Bearer TOKEN"
```

## Config Format

```toml
[server]
host = "0.0.0.0"
port = 8742
auth_token = "your-secret-token"

[agent]
name = "My Bridge"
description = "A2A bridge to Claude Code"
version = "0.1.0"

[agents.claude]
command = "/path/to/claude"
args = ["--input-format", "stream-json", "--output-format", "stream-json"]
workdir = "/path/to/workspace"
description = "Claude Code agent"
timeout = 600
keep_alive = true
# resume = "session-id"  # Uncomment to resume a session
```

## Session Management

- Sessions persist while bridge is running
- Use `/session` to get session ID
- Add `resume = "session-id"` in config to resume later
- Claude Code saves sessions on shutdown

## Auth

All endpoints except `/health` and Agent Card require:
```
Authorization: Bearer <token>
```

Set `auth_token` in config or remove for no auth.

## Endpoints

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/health` | GET | No | Health check |
| `/.well-known/agent-card.json` | GET | No | A2A Agent Card |
| `/a2a` | POST | Yes | Send message (JSON-RPC) |
| `/a2a?message=...` | GET | Yes | SSE streaming |
| `/session` | GET | Yes | Get session ID |
| `/shutdown` | POST | Yes | Stop bridge |

## Tips

- Keep `keep_alive = true` for multi-turn conversations
- Use `--dangerously-skip-permissions` for headless Claude Code
- Set `--setting-sources local` to use local Claude settings
- Binary is ~2.7MB, uses ~10MB RAM
