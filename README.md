# matrix-bridge

E2EE Matrix bridge — CLI and MCP server for any AI coding agent.

Send and read end-to-end encrypted messages in Matrix rooms from your terminal or any MCP-compatible tool (Claude Code, Cursor, Windsurf, Cline, etc.). Zero system dependencies — just download and run.

## Why

AI coding agents (Claude Code, Cursor, Windsurf, etc.) are powerful but isolated — they can't talk to other bots or people outside their own session. Matrix is an open, federated, end-to-end encrypted chat protocol that solves this.

With matrix-bridge, your coding agent can:

- **Collaborate with other AI agents** — have a coding agent build something, send it to a product agent for review, get feedback, and iterate — all automatically through encrypted chat
- **Join team rooms** — humans and bots in the same encrypted group chat, discussing work in real time
- **Bridge across providers** — an agent running on Anthropic can chat with one on OpenAI, or with a human on Element, all through the same Matrix room
- **Send and wait for replies** — ask a question to another bot or person and get the answer back in the same session

**Real-world example:** We use this to connect [Claude Code](https://claude.ai/code) with Žofka, a bot running on [Hermes Agent](https://github.com/hermes-agent/hermes-agent). Claude Code builds a plugin, Žofka tests it and reports issues, Claude reads her feedback and ships fixes — two agents working together in a contractor/supplier loop, with the human operator overseeing the conversation. The whole flow happens in an encrypted Matrix room via this bridge.

## Features

- **End-to-end encryption** — automatic encrypt/decrypt using vodozemac (pure Rust Olm)
- **CLI** — `matrix-bridge send`, `read`, `recent`, `rooms`, `send-wait`, `setup`, `config`
- **MCP server** — 6 tools exposed over stdio for any MCP client
- **Agent-agnostic** — no hardcoded bot names or provider assumptions
- **Static binaries** — no Python, no venv, no libolm
- **Cross-platform** — Linux (x86/ARM), macOS (Intel/Apple Silicon). Windows support planned.
- **TOFU trust** — Trust On First Use device verification, suitable for bot-to-bot communication

## Install

Currently from source only (GitHub Releases and crates.io coming after broader testing):

```bash
git clone https://github.com/elkimek/matrix-bridge.git
cd matrix-bridge
cargo build --release
# Binaries at target/release/matrix-bridge and target/release/matrix-bridge-mcp
```

## Quick start

### 1. Create a Matrix account

You need a Matrix account for the bridge. Create one at [element.io](https://app.element.io) or any Matrix homeserver, then note your user ID (e.g., `@mybot:matrix.org`).

### 2. Setup

```bash
matrix-bridge setup
# Enter your Matrix user ID; setup offers SSO and password when supported
# This creates encryption keys and saves your session
```

If this is a new device and you need older encrypted messages, restore the
account's room-key backup. The recovery key or passphrase is read locally and
is not saved by the bridge:

```bash
matrix-bridge recover
```

### 3. Configure a default room

```bash
# Use the config command to avoid shell escaping issues with !
matrix-bridge config default_room "!yourRoomId:matrix.org"
```

> **Tip:** Run `matrix-bridge rooms` to see your joined rooms and copy the room ID from there.

### 4. Send and read

```bash
# Send a message
matrix-bridge send "Hello from the bridge!"

# Read recent messages
matrix-bridge read --limit 10

# Read today's messages across all rooms
matrix-bridge recent --since 2026-09-29

# Send and wait for a reply (30s timeout)
matrix-bridge send-wait "ping" --timeout 30
```

## CLI reference

```
matrix-bridge setup                  Interactive login + key setup
matrix-bridge recover                Restore keys from the server backup
matrix-bridge send <msg>             Send a message
  --room <id>                        Room ID (overrides default)
  --mention <@user:server>           @mention a user
  --no-mention                       Suppress default mention
matrix-bridge read                   Read recent messages
  --room <id>                        Room ID (overrides default)
  --limit <n>                        Number of messages (1-100, default 10)
matrix-bridge recent                 Read all messages from all rooms since a cutoff
  --since <ts>                       RFC 3339 timestamp or YYYY-MM-DD (default: last 24h)
matrix-bridge rooms                  List joined rooms
matrix-bridge send-wait <msg>        Send and wait for reply
  --room <id>                        Room ID (overrides default)
  --mention <@user:server>           @mention a user
  --no-mention                       Suppress default mention
  --timeout <secs>                   Timeout (1-300, default 30)
matrix-bridge config                 View all config
matrix-bridge config <key>           View one config key
matrix-bridge config <key> <value>   Set a config key
matrix-bridge mcp-server             Start MCP server on stdio
```

All commands support `--json` for machine-readable output.

## MCP server

The MCP server exposes 6 tools over stdin/stdout:

| Tool | Description |
|------|-------------|
| `send_message` | Send a message to a room (auto-encrypted) |
| `send_and_wait` | Send and wait for a reply with timeout |
| `read_messages` | Read recent messages from one room (auto-decrypted) |
| `read_recent_messages` | Read ALL messages from all rooms since a cutoff (default: last 24h), no trimming |
| `list_rooms` | List joined rooms |
| `join_room` | Join a room by ID or alias |

### Claude Code

Add to `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "matrix": {
      "command": "/path/to/matrix-bridge-mcp"
    }
  }
}
```

### Cursor

Add to `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "matrix": {
      "command": "/path/to/matrix-bridge-mcp"
    }
  }
}
```

Works with any MCP client that supports stdio transport.

## Using with AI agent gateways

The bridge connects **coding agents** (Claude Code, Cursor, etc.) to Matrix. On the other side of the room you typically have a **gateway bot** — an always-on agent that lives in Matrix and responds to messages. The two main gateways are OpenClaw and Hermes Agent, and they handle Matrix natively (no bridge needed on their side).

### Hermes Agent

[Hermes Agent](https://github.com/hermes-agent/hermes-agent) connects to Matrix directly via its built-in gateway. No bridge needed on the Hermes side — just configure Matrix in `~/.hermes/.env`:

```bash
MATRIX_HOMESERVER=https://matrix.org
MATRIX_USER_ID=@yourbot:matrix.org
MATRIX_PASSWORD=your-password
MATRIX_ACCESS_TOKEN=your-token
MATRIX_ENCRYPTION=true
MATRIX_ALLOWED_USERS=@you:matrix.org
```

Then start the gateway:

```bash
hermes gateway start
```

**Group chat mention handling:** Hermes doesn't have built-in mention filtering for Matrix (unlike Discord/Telegram which have `require_mention`). In group rooms, the bot responds to every message. Two approaches:

1. **SOUL.md (soft gate)** — Add instructions to `~/.hermes/SOUL.md` telling the bot to distinguish between being addressed directly vs. mentioned in passing. The bot uses judgment — it may still respond to incidental mentions when it has something useful to add. This is often the better behavior for small team rooms.

2. **`require_mention` (hard gate)** — Not yet available for Matrix in Hermes. Planned as an upstream PR to bring parity with Discord/Telegram.

### OpenClaw

[OpenClaw](https://openclaw.com) also connects to Matrix natively via its gateway. Configure Matrix in `~/.openclaw/openclaw.json` and restart the gateway.

**Group chat mention handling:** OpenClaw supports [mention-gate](https://github.com/elkimek/mention-gate), a companion plugin that uses a cheap LLM (Haiku) to classify intent — "talking *to* the bot" vs. "talking *about* it" — and cancels replies to incidental mentions.

> **Security note:** mention-gate passes messages through an LLM classification prompt — prompt injection is a feature, not a bug (it's a noise filter, not a security boundary). Keep the gate's API key separate from the bot's main provider key. See [mention-gate's SECURITY.md](https://github.com/elkimek/mention-gate/blob/main/SECURITY.md) for details.

### The typical setup

```
 Your machine                                        Your server
┌──────────────────┐                                ┌──────────────────┐
│  Claude Code     │                                │  Hermes Agent    │
│  Cursor / Cline  │         Matrix Room            │  or OpenClaw     │
│  Windsurf / ...  │      ┌──────────────┐          │                  │
│                  │      │  E2EE group  │          │  ┌────────────┐  │
│  ┌────────────┐  │◄────►│  chat with   │◄────────►│  │  gateway   │  │
│  │ matrix-    │  │      │  humans and  │          │  │  (native   │  │
│  │ bridge     │  │      │  bots        │          │  │   Matrix)  │  │
│  └────────────┘  │      └──────┬───────┘          │  └────────────┘  │
└──────────────────┘             │                  └──────────────────┘
   coding agent                  │                     always-on bot
   builds, tests, ships    ┌─────┴──────┐             reviews, chats,
                           │  Element / │             runs tools
                           │  any Matrix│
                           │  client    │
                           └────────────┘
                            human operator
                            watches, steers
```

The bridge runs on the coding agent side. The gateway bot connects directly. Both meet in the same encrypted room, with the human operator joining via Element or any Matrix client.

## Configuration

Config lives at `~/.matrix-bridge/config.json` (created by `matrix-bridge setup`):

```json
{
  "homeserver": "https://matrix.org",
  "user_id": "@mybot:matrix.org",
  "device_name": "matrix-bridge",
  "store_path": "/home/user/.matrix-bridge/store",
  "trust_mode": "tofu",
  "default_room": "!roomid:matrix.org",
  "default_mention": "@friend:matrix.org",
  "notify_on_mention": "mybot"
}
```

| Field | Description |
|-------|-------------|
| `homeserver` | Matrix homeserver URL |
| `user_id` | Matrix user ID |
| `device_name` | Device display name (default: "matrix-bridge") |
| `store_path` | Path for encryption keys and state |
| `trust_mode` | Device trust: `tofu`, `all`, or `explicit` |
| `default_room` | Default room for CLI commands |
| `default_mention` | Default @mention for send commands |
| `notify_on_mention` | Pattern for MCP mention notifications (defaults to local part of user_id) |

### Trust modes

- **tofu** (default) — Trust On First Use. Auto-verify new devices. Best for bots.
- **all** — Trust all devices unconditionally.
- **explicit** — Manual verification only. Most secure, requires out-of-band verification.

## Troubleshooting

**Messages show as "[encrypted — unable to decrypt]"**

This means the bridge doesn't currently have the Megolm session keys for those messages. If the account has a server-side room-key backup, run `matrix-bridge recover` and enter its recovery key or passphrase. The bridge then requests missing room keys when you read messages.

If there is no backup, the existing Element device must share/export the room keys. Re-running `setup` creates another device but does not restore historical keys.

**"crypto store doesn't match" error during setup**

Delete the old crypto store and run setup again:
```bash
rm ~/.matrix-bridge/store/matrix-sdk-*.sqlite3 ~/.matrix-bridge/store/credentials.json
matrix-bridge setup
```

**Shell escaping issues with room IDs**

Room IDs start with `!` which bash interprets as history expansion. Use double quotes:
```bash
matrix-bridge config default_room "!roomid:matrix.org"
```

Or edit `~/.matrix-bridge/config.json` directly.

## Building

Requires Rust 1.80+ (for dependency compatibility).

```bash
cargo build --release
```

Feature flags:
- `cli` (default) — CLI binary with clap
- `mcp` (default) — MCP server with rmcp

Build CLI-only (no MCP):
```bash
cargo build --release --no-default-features --features cli
```

## License

GPL-3.0-or-later
