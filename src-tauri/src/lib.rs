mod claude;
mod db;
mod sources;
mod workflow;

use claude::{StreamEvent, TurnRequest};
use db::{Chat, Db, Message, NewMessage};
use serde::Serialize;
use sources::ToolPolicy;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;
use workflow::{Action, Event, Status};

const SETTING_CLAUDE_PATH: &str = "claude_path";
const SETTING_MODEL: &str = "model";
/// Team members' emails, one per line ("Name <email>" or just the email).
const SETTING_TEAM: &str = "team";
/// The app user's name, used as "Price Requested By" in the Supplier Notes.
const SETTING_USER_NAME: &str = "user_name";
/// Reference websites (train booking sites, ticket sites…) given to Claude with every search.
const SETTING_REFERENCE_LINKS: &str = "reference_links";
/// Comma-separated source ids ticked under the message box (remembered between searches).
const SETTING_ACTIVE_SOURCES: &str = "active_sources";
/// Per-source claude.ai connector name override: `connector_<source id>`.
const SETTING_CONNECTOR_PREFIX: &str = "connector_";

struct AppState {
    db: Mutex<Db>,
    /// Working dir for the Claude CLI. Sessions are stored per-cwd, so it must stay fixed.
    workspace: PathBuf,
    /// Running turns, keyed by chat id, so they can be stopped.
    running: Mutex<HashMap<String, oneshot::Sender<()>>>,
    /// Connectors from the last status check, used to keep unused ones out of each turn.
    connectors: Mutex<Vec<claude::Connector>>,
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
    state
        .db
        .lock()
        .unwrap()
        .create_chat("New chat")
        .map_err(err)
}

#[tauri::command]
fn rename_chat(state: State<AppState>, chat_id: String, title: String) -> CmdResult<()> {
    state
        .db
        .lock()
        .unwrap()
        .rename_chat(&chat_id, title.trim())
        .map_err(err)
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
    state
        .db
        .lock()
        .unwrap()
        .list_messages(&chat_id)
        .map_err(err)
}

// ---------- running a turn ----------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TurnFinished {
    chat_id: String,
    message: Option<Message>,
    cancelled: bool,
}

/// What the turn is for: a search, or a workflow step Claude performs.
#[derive(Clone, Copy)]
enum TurnKind {
    Search,
    Step(Action),
}

/// Runs one Claude turn in the background for `chat_id`.
/// Progress arrives as `claude://event`; the end as `claude://done`.
fn start_turn(
    app: AppHandle,
    state: &AppState,
    chat_id: String,
    prompt: String,
    policy: ToolPolicy,
    kind: TurnKind,
) -> CmdResult<()> {
    let (chat, claude_path, model) = {
        let db = state.db.lock().unwrap();
        (
            db.get_chat(&chat_id)
                .map_err(err)?
                .ok_or("Chat not found")?,
            db.get_setting(SETTING_CLAUDE_PATH).map_err(err)?,
            db.get_setting(SETTING_MODEL).map_err(err)?,
        )
    };
    let bin = claude::find_claude(claude_path.as_deref())
        .ok_or("Claude Code CLI not found. Install it, or set its path in Settings.")?;

    let (tx, rx) = oneshot::channel();
    {
        let mut running = state.running.lock().unwrap();
        if running.contains_key(&chat_id) {
            return Err("A reply is still being generated for this chat".into());
        }
        running.insert(chat_id.clone(), tx);
    }

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
                prompt: &prompt,
                resume_session: chat.claude_session_id.as_deref(),
                model: model.as_deref(),
                policy: &policy,
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
        let message = finish_turn(&db, &chat_id, kind, outcome);
        let cancelled = message
            .as_ref()
            .is_some_and(|m| m.content.ends_with("*(stopped)*"));
        drop(db);
        let _ = app.emit(
            "claude://done",
            TurnFinished {
                chat_id,
                message,
                cancelled,
            },
        );
    });
    Ok(())
}

/// Stores the turn's answer and moves the workflow forward.
fn finish_turn(
    db: &Db,
    chat_id: &str,
    kind: TurnKind,
    outcome: Result<claude::TurnResult, String>,
) -> Option<Message> {
    let res = match outcome {
        Ok(res) => res,
        Err(e) => return db.add_message(chat_id, NewMessage::new("error", &e)).ok(),
    };
    if let Some(sid) = &res.session_id {
        let _ = db.set_session_id(chat_id, sid);
    }
    let usage = res.usage.as_ref();

    if res.is_error {
        let detail = if res.text.trim().is_empty() {
            res.stderr.trim()
        } else {
            res.text.trim()
        };
        let text = format!("Claude returned an error: {detail}");
        return db
            .add_message(chat_id, NewMessage::new("error", &text).usage(usage))
            .ok();
    }

    let (content, search_outcome) = workflow::strip_outcome(&res.text);
    if content.is_empty() {
        return None;
    }
    let msg_kind = match kind {
        TurnKind::Search => "chat",
        TurnKind::Step(a) => a.output_kind(),
    };
    if res.cancelled {
        let text = format!("{content}\n\n*(stopped)*");
        return db
            .add_message(
                chat_id,
                NewMessage::new("assistant", &text)
                    .kind(msg_kind)
                    .usage(usage),
            )
            .ok();
    }

    let message = db
        .add_message(
            chat_id,
            NewMessage::new("assistant", &content)
                .kind(msg_kind)
                .usage(usage),
        )
        .ok();
    let event = match kind {
        TurnKind::Search => search_outcome,
        TurnKind::Step(a) => Some(a.done_event()),
    };
    if let Some(e) = event {
        let _ = apply_event(db, chat_id, e);
    }
    message
}

/// Applies a workflow event if the state machine allows it; returns the new status.
fn apply_event(db: &Db, chat_id: &str, event: Event) -> CmdResult<Option<Status>> {
    let chat = db.get_chat(chat_id).map_err(err)?.ok_or("Chat not found")?;
    let current = Status::parse(&chat.status);
    match workflow::next(current, event) {
        Some(next) => {
            db.set_status(chat_id, next.as_str()).map_err(err)?;
            Ok(Some(next))
        }
        None => Ok(None),
    }
}

fn source_policy(state: &AppState, db: &Db, source_ids: &[String]) -> ToolPolicy {
    let selected: Vec<sources::Selected> = source_ids
        .iter()
        .filter_map(|id| sources::get(id))
        .map(|def| sources::Selected {
            def,
            connector: def.default_connector.map(|_| connector_name(db, def)),
        })
        .collect();
    let connectors = state.connectors.lock().unwrap();
    sources::policy(&selected, &connectors, |id| {
        sources::local_files(&state.workspace, id)
    })
}

// ---------- searching ----------

/// Saves the user message, then searches the ticked sources in the background.
#[tauri::command]
fn send_message(
    app: AppHandle,
    state: State<AppState>,
    chat_id: String,
    text: String,
    sources: Vec<String>,
) -> CmdResult<Message> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Message is empty".into());
    }
    if state.running.lock().unwrap().contains_key(&chat_id) {
        return Err("A reply is still being generated for this chat".into());
    }
    let sources: Vec<String> = sources::SOURCES
        .iter()
        .filter(|d| sources.iter().any(|s| s == d.id))
        .map(|d| d.id.to_string())
        .collect();

    let (user_msg, policy) = {
        let db = state.db.lock().unwrap();
        db.get_chat(&chat_id)
            .map_err(err)?
            .ok_or("Chat not found")?;
        let is_first = db.list_messages(&chat_id).map_err(err)?.is_empty();
        let msg = db
            .add_message(&chat_id, NewMessage::new("user", &text).sources(&sources))
            .map_err(err)?;
        db.set_setting(SETTING_ACTIVE_SOURCES, &format!(",{}", sources.join(",")))
            .map_err(err)?;
        if is_first {
            db.rename_chat(&chat_id, &make_title(&text)).map_err(err)?;
        }
        let mut policy = source_policy(&state, &db, &sources);
        if let Some(links) = db
            .get_setting(SETTING_REFERENCE_LINKS)
            .map_err(err)?
            .filter(|l| !l.trim().is_empty())
        {
            policy.prompt.push_str(&format!(
                "\n## Reference links (train booking sites, ticket sites and other resources)\n\n{}\n",
                links.trim()
            ));
        }
        (msg, policy)
    };

    start_turn(app, &state, chat_id, text, policy, TurnKind::Search)?;
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

/// Chat title: for a request-form summary ("Service type: …", "Pick up location: …"),
/// "Service · From → To · Date"; otherwise the first line of the message.
fn make_title(text: &str) -> String {
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':').map(str::trim))
            .filter(|v| !v.is_empty())
    };
    let title = if let Some(service) = field("Service type") {
        let route = match (field("Pick up location"), field("Drop off location")) {
            (Some(a), Some(b)) => Some(format!("{a} → {b}")),
            (a, b) => a.or(b).map(str::to_string),
        };
        let when = field("Date").or_else(|| field("Approximate month"));
        [Some(service.to_string()), route, when.map(str::to_string)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    } else {
        text.lines().next().unwrap_or("").trim().to_string()
    };
    let mut short: String = title.chars().take(70).collect();
    if title.chars().count() > 70 {
        short.push('…');
    }
    if short.is_empty() {
        "New chat".into()
    } else {
        short
    }
}

#[cfg(test)]
mod title_tests {
    #[test]
    fn title_from_form_summary() {
        let t = super::make_title(
            "Brand: Tourist Italy\n\nDate: 18 May 2027\n\nService type: Transfer only\n\nPick up location: Matera\n\nDrop off location: Rome FCO",
        );
        assert_eq!(t, "Transfer only · Matera → Rome FCO · 18 May 2027");
        assert_eq!(
            super::make_title("2 pax Matera to Rome"),
            "2 pax Matera to Rome"
        );
    }
}

// ---------- workflow ----------

/// A step done by a person outside the app (sending an email, approving),
/// recorded in the chat and applied to the state machine.
#[tauri::command]
fn workflow_event(
    state: State<AppState>,
    chat_id: String,
    event: String,
    recipients: Vec<String>,
    approver: Option<String>,
    note: Option<String>,
) -> CmdResult<Vec<Message>> {
    let ev = Event::parse(&event).ok_or("Unknown workflow event")?;
    let approver = approver
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty());
    let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let db = state.db.lock().unwrap();
    let chat = db
        .get_chat(&chat_id)
        .map_err(err)?
        .ok_or("Chat not found")?;
    let current = Status::parse(&chat.status);
    let next = workflow::next(current, ev).ok_or_else(|| {
        format!(
            "This step isn't available while the request is '{}'",
            current.as_str()
        )
    })?;
    let to = recipients.join(", ");
    let text = match ev {
        Event::SentForApproval => format!("Sent for approval to {to}"),
        Event::Approved => {
            let by = approver.as_deref().unwrap_or("Operations");
            match &note {
                Some(n) => format!("Approved by {by}: {n}"),
                None => format!("Approved by {by}"),
            }
        }
        Event::Rejected => "Answer marked as not correct: ask the supplier".into(),
        Event::SentToOps => format!("Supplier message sent to Operations: {to}"),
        Event::SentToSales => format!("Final answer sent to {to}"),
        _ => event.clone(),
    };
    let msg = db
        .add_message(&chat_id, NewMessage::new("event", &text).kind(&event))
        .map_err(err)?;
    if ev == Event::Approved {
        db.set_approval(&chat_id, approver.as_deref().unwrap_or(""), note.as_deref())
            .map_err(err)?;
    }
    db.set_status(&chat_id, next.as_str()).map_err(err)?;
    Ok(vec![msg])
}

/// A step Claude performs: drafting the supplier message or writing the final answer.
/// For `final_from_supplier`, `supplier_answer` is recorded first and saved to "Saved answers".
#[tauri::command]
fn run_step(
    app: AppHandle,
    state: State<AppState>,
    chat_id: String,
    step: String,
    supplier_answer: Option<String>,
    approver: Option<String>,
) -> CmdResult<Vec<Message>> {
    let action = match step.as_str() {
        "draft_supplier" => Action::DraftSupplier,
        "final_from_approval" => Action::FinalFromApproval,
        "final_from_supplier" => Action::FinalFromSupplier,
        _ => return Err("Unknown step".into()),
    };
    if state.running.lock().unwrap().contains_key(&chat_id) {
        return Err("A reply is still being generated for this chat".into());
    }

    let mut added = Vec::new();
    let ctx = {
        let db = state.db.lock().unwrap();
        let chat = db
            .get_chat(&chat_id)
            .map_err(err)?
            .ok_or("Chat not found")?;
        let mut status = Status::parse(&chat.status);

        if action == Action::FinalFromSupplier {
            let answer = supplier_answer
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .ok_or("Paste the supplier's answer first")?;
            // B5: record the answer (unless it was already recorded and only the final answer is re-run).
            if let Some(next) = workflow::next(status, Event::SupplierAnswered) {
                let request = db
                    .list_messages(&chat_id)
                    .map_err(err)?
                    .into_iter()
                    .find(|m| m.role == "user")
                    .map(|m| m.content)
                    .unwrap_or_default();
                let file = workflow::save_answer(
                    &state.workspace,
                    &chat_id,
                    &chat.title,
                    &request,
                    answer,
                )
                .map_err(err)?;
                let text = format!(
                    "Supplier answer recorded and saved to **Saved answers** ({file}):\n\n> {}",
                    answer.replace('\n', "\n> ")
                );
                added.push(
                    db.add_message(
                        &chat_id,
                        NewMessage::new("event", &text).kind("supplier_answered"),
                    )
                    .map_err(err)?,
                );
                db.set_status(&chat_id, next.as_str()).map_err(err)?;
                status = next;
            }
            // The Operations person who forwarded the answer is the one approving it.
            if let Some(a) = approver.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
                db.set_approval(&chat_id, a, None).map_err(err)?;
            }
        }

        if !action.allowed_from(status) {
            return Err(format!(
                "This step isn't available while the request is '{}'",
                status.as_str()
            ));
        }
        added.push(
            db.add_message(
                &chat_id,
                NewMessage::new("user", action.label()).kind("action"),
            )
            .map_err(err)?,
        );
        let chat = db
            .get_chat(&chat_id)
            .map_err(err)?
            .ok_or("Chat not found")?;
        workflow::StepContext {
            requester: db.get_setting(SETTING_USER_NAME).map_err(err)?,
            approver: chat.approved_by.clone(),
            note: chat.approval_note.clone(),
            supplier_answer: supplier_answer.clone(),
            request_ref: format!(
                "Pricy request \"{}\" ({}, ID {})",
                chat.title,
                workflow::date_of(chat.created_at),
                &chat.id[..8.min(chat.id.len())]
            ),
        }
    };

    let prompt = action.prompt(&ctx);
    start_turn(
        app,
        &state,
        chat_id,
        prompt,
        sources::policy_no_tools(&state.connectors.lock().unwrap()),
        TurnKind::Step(action),
    )?;
    Ok(added)
}

// ---------- data sources ----------

fn connector_name(db: &Db, def: &sources::SourceDef) -> String {
    db.get_setting(&format!("{SETTING_CONNECTOR_PREFIX}{}", def.id))
        .ok()
        .flatten()
        .or(def.default_connector.map(str::to_string))
        .unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceInfo {
    #[serde(flatten)]
    def: sources::SourceDef,
    /// claude.ai connector used for this source (None for local sources).
    connector: Option<String>,
    active: bool,
    /// Number of files, for local sources (WhatsApp, Saved answers).
    files: Option<usize>,
}

#[tauri::command]
fn list_sources(state: State<AppState>) -> CmdResult<Vec<SourceInfo>> {
    let db = state.db.lock().unwrap();
    // First run: everything ticked.
    let active = db.get_setting(SETTING_ACTIVE_SOURCES).map_err(err)?;
    Ok(sources::SOURCES
        .iter()
        .map(|def| SourceInfo {
            def: *def,
            connector: def.default_connector.map(|_| connector_name(&db, def)),
            active: active
                .as_deref()
                .is_none_or(|a| a.split(',').any(|id| id == def.id)),
            files: sources::local_rel_dir(def.id)
                .map(|_| sources::local_files(&state.workspace, def.id).len()),
        })
        .collect())
}

#[tauri::command]
fn set_active_sources(state: State<AppState>, sources: Vec<String>) -> CmdResult<()> {
    state
        .db
        .lock()
        .unwrap()
        // Never store an empty string: that would be read back as "first run".
        .set_setting(SETTING_ACTIVE_SOURCES, &format!(",{}", sources.join(",")))
        .map_err(err)
}

#[tauri::command]
fn set_connector(state: State<AppState>, source_id: String, connector: String) -> CmdResult<()> {
    let def = sources::get(&source_id).ok_or("Unknown source")?;
    let db = state.db.lock().unwrap();
    let key = format!("{SETTING_CONNECTOR_PREFIX}{}", def.id);
    // Empty or default → fall back to the default name.
    let value = if Some(connector.trim()) == def.default_connector {
        ""
    } else {
        connector.trim()
    };
    db.set_setting(&key, value).map_err(err)
}

#[tauri::command]
async fn list_connectors(state: State<'_, AppState>) -> CmdResult<Vec<claude::Connector>> {
    let override_path = state
        .db
        .lock()
        .unwrap()
        .get_setting(SETTING_CLAUDE_PATH)
        .map_err(err)?;
    let bin = claude::find_claude(override_path.as_deref()).ok_or("Claude Code CLI not found")?;
    let list = claude::list_connectors(&bin, &state.workspace).await?;
    *state.connectors.lock().unwrap() = list.clone();
    Ok(list)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceFile {
    name: String,
    size: u64,
    modified_at: i64,
}

fn list_whatsapp_names(workspace: &std::path::Path) -> Vec<String> {
    sources::local_files(workspace, "whatsapp")
}

#[tauri::command]
fn list_whatsapp_files(state: State<AppState>) -> Vec<SourceFile> {
    let dir = sources::whatsapp_dir(&state.workspace);
    list_whatsapp_names(&state.workspace)
        .into_iter()
        .filter_map(|name| {
            let meta = std::fs::metadata(dir.join(&name)).ok()?;
            let modified_at = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            Some(SourceFile {
                name,
                size: meta.len(),
                modified_at,
            })
        })
        .collect()
}

/// Copies the picked WhatsApp exports into the app's workspace, so they stay
/// available (and searchable by Claude) even if the originals move.
#[tauri::command]
fn add_whatsapp_files(state: State<AppState>, paths: Vec<String>) -> CmdResult<Vec<String>> {
    let dir = sources::whatsapp_dir(&state.workspace);
    std::fs::create_dir_all(&dir).map_err(err)?;
    let mut added = Vec::new();
    for p in paths {
        let src = PathBuf::from(&p);
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("Invalid file: {p}"))?;
        // Exports from different chats often share the name "_chat.txt": prefix the parent folder.
        let name = if name.eq_ignore_ascii_case("_chat.txt") {
            let parent = src
                .parent()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("WhatsApp");
            format!("{parent}.txt")
        } else {
            name.to_string()
        };
        std::fs::copy(&src, dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
        added.push(name);
    }
    Ok(added)
}

#[tauri::command]
fn remove_whatsapp_file(state: State<AppState>, name: String) -> CmdResult<()> {
    if name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("Invalid file name".into());
    }
    std::fs::remove_file(sources::whatsapp_dir(&state.workspace).join(name)).map_err(err)
}

// ---------- settings ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    claude_path: String,
    model: String,
    /// One team member per line: "Name <email>" or "email".
    team: String,
    user_name: String,
    reference_links: String,
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
        claude_path: db
            .get_setting(SETTING_CLAUDE_PATH)
            .map_err(err)?
            .unwrap_or_default(),
        model: db
            .get_setting(SETTING_MODEL)
            .map_err(err)?
            .unwrap_or_default(),
        team: db
            .get_setting(SETTING_TEAM)
            .map_err(err)?
            .unwrap_or_default(),
        user_name: db
            .get_setting(SETTING_USER_NAME)
            .map_err(err)?
            .unwrap_or_default(),
        reference_links: db
            .get_setting(SETTING_REFERENCE_LINKS)
            .map_err(err)?
            .unwrap_or_default(),
    })
}

#[tauri::command]
fn save_settings(
    state: State<AppState>,
    claude_path: String,
    model: String,
    team: String,
    user_name: String,
    reference_links: String,
) -> CmdResult<()> {
    let db = state.db.lock().unwrap();
    db.set_setting(SETTING_TEAM, team.trim()).map_err(err)?;
    db.set_setting(SETTING_USER_NAME, user_name.trim())
        .map_err(err)?;
    db.set_setting(SETTING_REFERENCE_LINKS, reference_links.trim())
        .map_err(err)?;
    db.set_setting(SETTING_CLAUDE_PATH, claude_path.trim())
        .map_err(err)?;
    db.set_setting(SETTING_MODEL, model.trim()).map_err(err)?;
    Ok(())
}

#[tauri::command]
async fn check_claude(state: State<'_, AppState>) -> CmdResult<ClaudeStatus> {
    let override_path = state
        .db
        .lock()
        .unwrap()
        .get_setting(SETTING_CLAUDE_PATH)
        .map_err(err)?;
    let Some(bin) = claude::find_claude(override_path.as_deref()) else {
        return Ok(ClaudeStatus {
            path: None,
            version: None,
            error: Some("Claude Code CLI not found".into()),
        });
    };
    let path = Some(bin.display().to_string());
    Ok(match claude::claude_version(&bin).await {
        Ok(v) => ClaudeStatus {
            path,
            version: Some(v),
            error: None,
        },
        Err(e) => ClaudeStatus {
            path,
            version: None,
            error: Some(e),
        },
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
                connectors: Mutex::new(Vec::new()),
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
            list_sources,
            set_active_sources,
            set_connector,
            list_connectors,
            list_whatsapp_files,
            add_whatsapp_files,
            remove_whatsapp_file,
            workflow_event,
            run_step,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
