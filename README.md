# Pricy

Internal desktop tool for Sales / OPS to find quote prices quickly, using the company's shared data.

**Data sources:** TINT, Slack and Google Drive through the user's own claude.ai connectors (read-only), plus WhatsApp chat exports (`.txt`) attached in Settings. Sales ticks which sources to search under the message box.
**Covers:** transfers and tours.
**Output:** a short answer with the price, timing (travel time or tour duration), conditions and the source it came from.

**Workflow:** price found → approval by Operations → final answer to Sales; or no price → supplier message via Operations → supplier answer saved → final answer to Sales. Team emails are set in Settings; messages open in the mail app.
**Token usage** is shown under every answer and as a total per request.

Architecture and data flow: [architecture/ARCHITECTURE.md](architecture/ARCHITECTURE.md). Workflow state machines: [architecture/WORKFLOW.md](architecture/WORKFLOW.md).

## How it works

```
React UI (src/)  ──invoke/events──▶  Rust core (src-tauri/)  ──spawns──▶  claude -p (installed Claude Code CLI)
                                      └─ SQLite: chats, messages, settings
```

- The app reuses the **Claude Code CLI already installed** on the computer and the user's own Claude login. No API key is stored in the app.
- Each chat maps to a Claude session (`--resume <session_id>`), so follow-up questions keep context.
- The CLI runs isolated from the user's personal Claude Code settings (`--setting-sources ""`). Per search, only read-type tools of the ticked sources are allowed; see [src-tauri/src/sources.rs](src-tauri/src/sources.rs).
- TINT, Slack and Drive must be connected on claude.ai (Settings → Connectors). Their status is shown next to each source.
- The assistant's instructions are in [src-tauri/src/system_prompt.md](src-tauri/src/system_prompt.md).
- Local data is stored in `~/Library/Application Support/com.coax.pricy/` (`pricy.db`, plus `workspace/` as the CLI working dir).

## Requirements

- Node 20+ and pnpm
- Rust (stable) and Xcode Command Line Tools on macOS
- [Claude Code](https://docs.claude.com/en/docs/claude-code) installed and logged in (`claude` → `/login`)

## Run

```bash
pnpm install
pnpm tauri dev      # development
pnpm tauri build    # release bundle (.app / .dmg)
```

If the app can't find `claude`, set the CLI path in **Settings**. The app checks PATH, `~/.local/bin`, Homebrew, and the VS Code extension's bundled binary.

## Structure

| Path | What |
|---|---|
| `src/App.tsx` | Chat UI: history sidebar, streaming replies, composer |
| `src/SettingsDialog.tsx` | Data sources (connectors, WhatsApp files), Claude CLI path, model |
| `src/useSources.ts` | Source list, ticked sources, connector status |
| `src/Workflow.tsx` | Workflow bar, send dialog (team checkboxes → mail), supplier answer dialog, token usage |
| `src/api.ts` | Typed wrappers for Tauri commands and events |
| `src-tauri/src/lib.rs` | Tauri commands, app state |
| `src-tauri/src/claude.rs` | Finds and runs the Claude CLI, parses `stream-json`, lists connectors |
| `src-tauri/src/sources.rs` | Source definitions and the per-search tool policy |
| `src-tauri/src/workflow.rs` | Request state machine, Claude workflow steps, Saved answers |
| `src-tauri/src/db.rs` | SQLite schema and queries |
