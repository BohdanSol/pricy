# Pricy

Internal desktop tool for Sales / OPS to find quote prices quickly, using the company's shared data.

**Data sources (planned):** TINT (MCP), Slack, WhatsApp chat export (`.txt`, picked from disk), Google Drive folders with Excel price tables.
**Output:** a short answer with the price, its conditions and the source it came from.

## How it works

```
React UI (src/)  ──invoke/events──▶  Rust core (src-tauri/)  ──spawns──▶  claude -p (installed Claude Code CLI)
                                      └─ SQLite: chats, messages, settings
```

- The app reuses the **Claude Code CLI already installed** on the computer and the user's own Claude login. No API key is stored in the app.
- Each chat maps to a Claude session (`--resume <session_id>`), so follow-up questions keep context.
- The CLI runs isolated from the user's personal Claude Code setup (`--setting-sources ""`, `--strict-mcp-config`, `--tools ""`). Data-source tools will be added explicitly.
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
| `src/SettingsDialog.tsx` | Claude CLI path + model |
| `src/api.ts` | Typed wrappers for Tauri commands and events |
| `src-tauri/src/lib.rs` | Tauri commands, app state |
| `src-tauri/src/claude.rs` | Finds and runs the Claude CLI, parses `stream-json` |
| `src-tauri/src/db.rs` | SQLite schema and queries |
