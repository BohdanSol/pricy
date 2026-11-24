mod claude;
mod db;

use claude::{StreamEvent, TurnRequest};
use db::{Chat, Db, Message};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

const SETTING_CLAUDE_PATH: &str = "claude_path";
const SETTING_MODEL: &str = "model";

struct AppState {
    db: Mutex<Db>,
    /// Working dir for the Claude CLI. Sessions are stored per-cwd, so it must stay fixed.
    workspace: PathBuf,
    /// Running turns, keyed by chat id, so they can be stopped.
    running: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------- chats ----------

#[tauri::command]
fn list_chats(state: State<AppState>) -> CmdResult<Vec<Chat>> {
    state.db.lock().unwrap().list_chats().map_err(err)
}

#[tauri::command]
fn create_chat(state: State<AppState>) -> CmdResult<Chat> {
    state.db.lock().unwrap().create_chat("New chat").map_err(err)
}

#[tauri::command]
fn rename_chat(state: State<AppState>, chat_id: String, title: String) -> CmdResult<()> {
    state.db.lock().unwrap().rename_chat(&chat_id, title.trim()).map_err(err)
}

#[tauri::command]
fn delete_chat(state: State<AppState>, chat_id: String) -> CmdResult<()> {
    if let Some(tx) = state.running.lock().unwrap().remove(&chat_id) {
        let _ = tx.send(());
    }
    state.db.lock().unwrap().delete_chat(&chat_id).map_err(err)
}

#[tauri::command]
fn get_messages(state: State<AppState>, chat_id: String) -> CmdResult<Vec<Message>> {
    state.db.lock().unwrap().list_messages(&chat_id).map_err(err)
}

// ---------- sending ----------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TurnFinished {
    chat_id: String,
    message: Option<Message>,
    cancelled: bool,
}

/// Saves the user message, then runs the turn in the background.
/// Progress arrives as `claude://event`; the end as `claude://done`.
#[tauri::command]
fn send_message(
    app: AppHandle,
    state: State<AppState>,
    chat_id: String,
    text: String,
) -> CmdResult<Message> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Message is empty".into());
    }
    if state.running.lock().unwrap().contains_key(&chat_id) {
        return Err("A reply is still being generated for this chat".into());
    }

    let (user_msg, chat, claude_path, model) = {
        let db = state.db.lock().unwrap();
        let chat = db.get_chat(&chat_id).map_err(err)?.ok_or("Chat not found")?;
        let is_first = db.list_messages(&chat_id).map_err(err)?.is_empty();
        let msg = db.add_message(&chat_id, "user", &text).map_err(err)?;
        if is_first {
            db.rename_chat(&chat_id, &make_title(&text)).map_err(err)?;
        }
        (
            msg,
            chat,
            db.get_setting(SETTING_CLAUDE_PATH).map_err(err)?,
            db.get_setting(SETTING_MODEL).map_err(err)?,
        )
    };

    let bin = claude::find_claude(claude_path.as_deref()).ok_or(
        "Claude Code CLI not found. Install it, or set its path in Settings.",
    )?;

    let (tx, rx) = oneshot::channel();
    state.running.lock().unwrap().insert(chat_id.clone(), tx);

    let workspace = state.workspace.clone();
    tauri::async_runtime::spawn(async move {
        let emit_app = app.clone();
        let on_event = move |ev: StreamEvent| {
            let _ = emit_app.emit("claude://event", ev);
        };

        let outcome = claude::run_turn(
            TurnRequest {
                bin: &bin,
                cwd: &workspace,
                prompt: &text,
                resume_session: chat.claude_session_id.as_deref(),
                model: model.as_deref(),
            },
            &chat_id,
            rx,
            on_event,
        )
        .await;

        let state = app.state::<AppState>();
        state.running.lock().unwrap().remove(&chat_id);

        let db = state.db.lock().unwrap();
        // Chat may have been deleted while the reply was generating.
        if db.get_chat(&chat_id).ok().flatten().is_none() {
            return;
        }

        let finished = match outcome {
            Ok(res) => {
                if let Some(sid) = &res.session_id {
                    let _ = db.set_session_id(&chat_id, sid);
                }
                let (role, content) = if res.is_error {
                    let detail = if res.text.trim().is_empty() {
                        res.stderr.trim().to_string()
                    } else {
                        res.text.trim().to_string()
                    };
                    ("error", format!("Claude returned an error: {detail}"))
                } else {
                    ("assistant", res.text.trim().to_string())
                };
                let message = if content.is_empty() {
                    None
                } else if res.cancelled {
                    db.add_message(&chat_id, "assistant", &format!("{content}\n\n*(stopped)*"))
                        .ok()
                } else {
                    db.add_message(&chat_id, role, &content).ok()
                };
                TurnFinished { chat_id: chat_id.clone(), message, cancelled: res.cancelled }
            }
            Err(e) => TurnFinished {
                chat_id: chat_id.clone(),
                message: db.add_message(&chat_id, "error", &e).ok(),
                cancelled: false,
            },
        };
        drop(db);
        let _ = app.emit("claude://done", finished);
    });

    Ok(user_msg)
}

#[tauri::command]
fn stop_message(state: State<AppState>, chat_id: String) {
    if let Some(tx) = state.running.lock().unwrap().remove(&chat_id) {
        let _ = tx.send(());
    }
}

#[tauri::command]
fn running_chats(state: State<AppState>) -> Vec<String> {
    state.running.lock().unwrap().keys().cloned().collect()
}

fn make_title(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    if title.is_empty() { "New chat".into() } else { title }
}

// ---------- settings ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    claude_path: String,
    model: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeStatus {
    path: Option<String>,
    version: Option<String>,
    error: Option<String>,
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> CmdResult<Settings> {
    let db = state.db.lock().unwrap();
    Ok(Settings {
        claude_path: db.get_setting(SETTING_CLAUDE_PATH).map_err(err)?.unwrap_or_default(),
        model: db.get_setting(SETTING_MODEL).map_err(err)?.unwrap_or_default(),
    })
}

#[tauri::command]
fn save_settings(state: State<AppState>, claude_path: String, model: String) -> CmdResult<()> {
    let db = state.db.lock().unwrap();
    db.set_setting(SETTING_CLAUDE_PATH, claude_path.trim()).map_err(err)?;
    db.set_setting(SETTING_MODEL, model.trim()).map_err(err)?;
    Ok(())
}

#[tauri::command]
async fn check_claude(state: State<'_, AppState>) -> CmdResult<ClaudeStatus> {
    let override_path = state.db.lock().unwrap().get_setting(SETTING_CLAUDE_PATH).map_err(err)?;
    let Some(bin) = claude::find_claude(override_path.as_deref()) else {
        return Ok(ClaudeStatus {
            path: None,
            version: None,
            error: Some("Claude Code CLI not found".into()),
        });
    };
    let path = Some(bin.display().to_string());
    Ok(match claude::claude_version(&bin).await {
        Ok(v) => ClaudeStatus { path, version: Some(v), error: None },
        Err(e) => ClaudeStatus { path, version: None, error: Some(e) },
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let workspace = data_dir.join("workspace");
            std::fs::create_dir_all(&workspace)?;
            let db = Db::open(&data_dir.join("pricy.db"))?;
            app.manage(AppState {
                db: Mutex::new(db),
                workspace,
                running: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_chats,
            create_chat,
            rename_chat,
            delete_chat,
            get_messages,
            send_message,
            stop_message,
            running_chats,
            get_settings,
            save_settings,
            check_claude,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
