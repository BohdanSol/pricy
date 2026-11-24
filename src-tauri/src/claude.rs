//! Runs chat turns through the Claude Code CLI already installed on the machine
//! (`claude -p --output-format stream-json`), so the app reuses the user's own
//! Claude login. No API key is stored or shipped with the app.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;

pub const SYSTEM_PROMPT: &str = include_str!("system_prompt.md");

/// Locate the `claude` binary. GUI apps on macOS don't inherit the shell PATH,
/// so we also check the usual install locations.
pub fn find_claude(override_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = override_path.filter(|p| !p.trim().is_empty()) {
        let p = PathBuf::from(p.trim());
        return p.is_file().then_some(p);
    }

    let exe = if cfg!(windows) { "claude.exe" } else { "claude" };
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join(exe)));
    }
    if let Some(home) = home_dir() {
        candidates.push(home.join(".local/bin").join(exe));
        candidates.push(home.join(".claude/local").join(exe));
        candidates.push(home.join(".npm-global/bin").join(exe));
        if cfg!(windows) {
            candidates.push(home.join("AppData/Roaming/npm/claude.cmd"));
        }
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin").join(exe));
    candidates.push(PathBuf::from("/usr/local/bin").join(exe));

    if let Some(p) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }

    // Fallback: the binary bundled with the VS Code extension (newest version).
    let ext_dir = home_dir()?.join(".vscode/extensions");
    let mut exts: Vec<PathBuf> = std::fs::read_dir(ext_dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("anthropic.claude-code-"))
        })
        .collect();
    exts.sort();
    exts.into_iter()
        .rev()
        .map(|p| p.join("resources/native-binary").join(exe))
        .find(|p| p.is_file())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub async fn claude_version(bin: &Path) -> Result<String, String> {
    let out = Command::new(bin)
        .arg("--version")
        .output()
        .await
        .map_err(|e| format!("failed to run {}: {e}", bin.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Streamed to the UI as `claude://event`.
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StreamEvent {
    /// A chunk of assistant text.
    Delta { chat_id: String, text: String },
    /// Claude started using a tool (data source lookups, later on).
    Tool { chat_id: String, name: String, input: Value },
}

pub struct TurnRequest<'a> {
    pub bin: &'a Path,
    pub cwd: &'a Path,
    pub prompt: &'a str,
    pub resume_session: Option<&'a str>,
    pub model: Option<&'a str>,
}

#[derive(Debug, Default)]
pub struct TurnResult {
    pub session_id: Option<String>,
    pub text: String,
    pub is_error: bool,
    pub cancelled: bool,
    pub stderr: String,
}

/// Run one turn. `on_event` is called for every streamed delta/tool use.
pub async fn run_turn(
    req: TurnRequest<'_>,
    chat_id: &str,
    mut cancel: oneshot::Receiver<()>,
    on_event: impl Fn(StreamEvent),
) -> Result<TurnResult, String> {
    let mut cmd = Command::new(req.bin);
    cmd.current_dir(req.cwd)
        .args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            // Isolate from the user's personal Claude Code config (CLAUDE.md, hooks, MCPs).
            "--setting-sources",
            "",
            "--strict-mcp-config",
            // No built-in tools yet: data-source tools get added here later.
            "--tools",
            "",
            "--system-prompt",
            SYSTEM_PROMPT,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(model) = req.model.filter(|m| !m.is_empty()) {
        cmd.args(["--model", model]);
    }
    if let Some(sid) = req.resume_session {
        cmd.args(["--resume", sid]);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to start {}: {e}", req.bin.display()))?;

    // Prompt goes through stdin to avoid any argument-escaping issues.
    let mut stdin = child.stdin.take().expect("stdin piped");
    stdin
        .write_all(req.prompt.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    drop(stdin);

    let mut stderr = child.stderr.take().expect("stderr piped");
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf).await;
        buf
    });

    let mut lines = BufReader::new(child.stdout.take().expect("stdout piped")).lines();
    let mut result = TurnResult::default();
    let mut streamed = String::new();

    loop {
        tokio::select! {
            _ = &mut cancel => {
                let _ = child.kill().await;
                result.cancelled = true;
                break;
            }
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                handle_line(&v, chat_id, &mut result, &mut streamed, &on_event);
            }
        }
    }

    let status = child.wait().await.ok();
    result.stderr = stderr_task.await.unwrap_or_default();
    if result.text.is_empty() {
        result.text = streamed;
    }
    if !result.cancelled && status.is_some_and(|s| !s.success()) {
        result.is_error = true;
    }
    Ok(result)
}

fn handle_line(
    v: &Value,
    chat_id: &str,
    result: &mut TurnResult,
    streamed: &mut String,
    on_event: &impl Fn(StreamEvent),
) {
    if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
        result.session_id = Some(sid.to_string());
    }
    match v.get("type").and_then(Value::as_str) {
        Some("stream_event") => {
            let ev = &v["event"];
            // A new model message after a tool call: separate it from the previous text.
            if ev["type"] == "message_start" && !streamed.is_empty() && !streamed.ends_with("\n\n") {
                streamed.push_str("\n\n");
                on_event(StreamEvent::Delta {
                    chat_id: chat_id.to_string(),
                    text: "\n\n".into(),
                });
            }
            if ev["type"] == "content_block_delta" && ev["delta"]["type"] == "text_delta" {
                if let Some(t) = ev["delta"]["text"].as_str() {
                    streamed.push_str(t);
                    on_event(StreamEvent::Delta {
                        chat_id: chat_id.to_string(),
                        text: t.to_string(),
                    });
                }
            }
        }
        Some("assistant") => {
            if let Some(blocks) = v["message"]["content"].as_array() {
                for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    on_event(StreamEvent::Tool {
                        chat_id: chat_id.to_string(),
                        name: b["name"].as_str().unwrap_or("tool").to_string(),
                        input: b["input"].clone(),
                    });
                }
            }
        }
        Some("result") => {
            result.is_error = v["is_error"].as_bool().unwrap_or(false);
            if let Some(t) = v["result"].as_str() {
                result.text = t.to_string();
            }
        }
        _ => {}
    }
}
