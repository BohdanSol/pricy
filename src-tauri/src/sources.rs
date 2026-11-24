//! Data sources the user can search, and how each one is exposed to Claude.
//!
//! - TINT, Slack, Drive: the user's own claude.ai connectors, which the Claude
//!   Code CLI loads on demand. Only read-type tools are allowed (by name pattern),
//!   so Pricy can never post, book or create anything.
//! - WhatsApp: chat export `.txt` files copied into `<workspace>/sources/whatsapp`,
//!   searched with the CLI's built-in read-only Grep/Read/Glob tools.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct SourceDef {
    pub id: &'static str,
    pub label: &'static str,
    /// Default claude.ai connector name (as `claude mcp list` prints it). None = local source.
    pub default_connector: Option<&'static str>,
    pub hint: &'static str,
}

pub const SOURCES: &[SourceDef] = &[
    SourceDef {
        id: "tint",
        label: "TINT",
        default_connector: Some("claude.ai TINT-staging"),
        hint: "Bookings, services (transfers, tours) and prices in the touring platform. Usually the most authoritative source for existing prices.",
    },
    SourceDef {
        id: "slack",
        label: "Slack",
        default_connector: Some("claude.ai Slack"),
        hint: "Team conversations where prices and supplier answers were discussed. Search by route, tour or place names, supplier or booking number.",
    },
    SourceDef {
        id: "whatsapp",
        label: "WhatsApp",
        default_connector: None,
        hint: "Exported WhatsApp chats with suppliers (plain text, one message per line starting with a date).",
    },
    SourceDef {
        id: "drive",
        label: "Drive",
        default_connector: Some("claude.ai Google Drive"),
        hint: "Google Drive folders with price tables (Excel / Google Sheets). Search for spreadsheets, then read the matching rows.",
    },
];

pub fn get(id: &str) -> Option<&'static SourceDef> {
    SOURCES.iter().find(|s| s.id == id)
}

/// Tool-name patterns treated as read-only. Applied per connector.
const READ_PATTERNS: &[&str] = &[
    "*search*", "*get_*", "*list*", "*read*", "*find*", "*fetch*", "*query*", "*download*", "*lookup*",
];

/// Built-in Claude Code tools Pricy never needs. Blocked explicitly, because
/// restricting the built-in set with `--tools` also hides the claude.ai connectors.
pub const BLOCKED_BUILTINS: &[&str] = &[
    "Bash", "Edit", "Write", "NotebookEdit", "WebFetch", "WebSearch", "Task", "Agent", "Skill",
    "Workflow", "SendMessage", "RemoteTrigger", "PushNotification", "Monitor", "ScheduleWakeup",
    "CronCreate", "CronDelete", "CronList", "EnterWorktree", "ExitWorktree", "DesignSync",
    "ShareOnboardingGuide", "ReportFindings", "TaskCreate", "TaskUpdate", "TaskStop", "TaskOutput",
    "TaskGet", "TaskList", "TodoWrite", "KillShell", "BashOutput", "ExitPlanMode", "EnterPlanMode",
    "AskUserQuestion",
];

const LOCAL_READ_TOOLS: &[&str] = &["Read", "Grep", "Glob"];

/// "claude.ai Google Drive" → "mcp__claude_ai_Google_Drive"
pub fn tool_prefix(connector: &str) -> String {
    let norm: String = connector
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    format!("mcp__{norm}")
}

pub fn whatsapp_dir(workspace: &Path) -> PathBuf {
    workspace.join("sources").join("whatsapp")
}

/// A source the user selected for this search, with its resolved connector name.
pub struct Selected {
    pub def: &'static SourceDef,
    pub connector: Option<String>,
}

pub struct ToolPolicy {
    pub allowed: Vec<String>,
    pub disallowed: Vec<String>,
    /// Appended to the system prompt for this turn.
    pub prompt: String,
}

pub fn policy(selected: &[Selected], whatsapp_files: &[String]) -> ToolPolicy {
    let mut allowed: Vec<String> = vec!["ToolSearch".into()];
    let mut disallowed: Vec<String> = BLOCKED_BUILTINS.iter().map(|s| s.to_string()).collect();
    let mut prompt = String::from("## Sources enabled for this search\n\n");

    if selected.is_empty() {
        prompt.push_str(
            "No sources are enabled. Tell the user to tick at least one source (TINT, Slack, WhatsApp, Drive) under the message box. Do not use any tools.\n",
        );
    }

    let mut uses_local = false;
    for s in selected {
        prompt.push_str(&format!("- **{}**: {}\n", s.def.label, s.def.hint));
        match (&s.connector, s.def.id) {
            (Some(conn), _) => {
                let prefix = tool_prefix(conn);
                allowed.extend(READ_PATTERNS.iter().map(|p| format!("{prefix}__{p}")));
                prompt.push_str(&format!(
                    "  Tools: `{prefix}__*` (load them with ToolSearch, e.g. query \"{}\"). Read-only: anything that writes or sends will be denied.\n",
                    s.def.label.to_lowercase()
                ));
            }
            (None, "whatsapp") => {
                uses_local = true;
                if whatsapp_files.is_empty() {
                    prompt.push_str("  No WhatsApp export is attached yet: tell the user to attach one in Settings.\n");
                } else {
                    prompt.push_str(&format!(
                        "  Files (relative to the working directory): {}.\n  Use Grep with case-insensitive patterns (try place-name variants and abbreviations, e.g. \"FCO|Fiumicino\"), then Read around the matches for prices and dates. Quote the chat name, message date and sender as the source.\n",
                        whatsapp_files
                            .iter()
                            .map(|f| format!("`sources/whatsapp/{f}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            _ => {}
        }
    }

    if uses_local {
        allowed.extend(LOCAL_READ_TOOLS.iter().map(|s| s.to_string()));
    } else {
        disallowed.extend(LOCAL_READ_TOOLS.iter().map(|s| s.to_string()));
    }

    prompt.push_str(
        "\nSearch only the sources listed above; any other tool will be denied. If a source fails (for example it needs re-authentication), say which one and continue with the others.\n",
    );

    ToolPolicy { allowed, disallowed, prompt }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_normalises_connector_names() {
        assert_eq!(tool_prefix("claude.ai Google Drive"), "mcp__claude_ai_Google_Drive");
        assert_eq!(tool_prefix("claude.ai TINT-staging"), "mcp__claude_ai_TINT-staging");
    }

    #[test]
    fn local_tools_only_when_whatsapp_selected() {
        let drive = Selected { def: get("drive").unwrap(), connector: Some("claude.ai Google Drive".into()) };
        let p = policy(&[drive], &[]);
        assert!(p.allowed.iter().any(|a| a == "mcp__claude_ai_Google_Drive__*search*"));
        assert!(p.disallowed.iter().any(|d| d == "Read"));

        let wa = Selected { def: get("whatsapp").unwrap(), connector: None };
        let p = policy(&[wa], &["chat.txt".into()]);
        assert!(p.allowed.iter().any(|a| a == "Grep"));
        assert!(!p.disallowed.iter().any(|d| d == "Grep"));
        assert!(p.prompt.contains("sources/whatsapp/chat.txt"));
    }
}
