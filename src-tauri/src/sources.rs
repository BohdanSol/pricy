//! Data sources the user can search, and how each one is exposed to Claude.
//!
//! - TINT, Slack, Drive: the user's own claude.ai connectors, which the Claude
//!   Code CLI loads on demand. Only read-type tools are allowed (by name pattern),
//!   so Pricy can never post, book or create anything.
//! - WhatsApp: chat export `.txt` files copied into `<workspace>/sources/whatsapp`,
//!   searched with the CLI's built-in read-only Grep/Read/Glob tools.

use crate::claude::Connector;
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
        id: "saved",
        label: "Saved answers",
        default_connector: None,
        hint: "Supplier answers recorded in Pricy for earlier requests (Markdown files, one per request). Already confirmed by a supplier, so usually the most reliable recent price.",
    },
    SourceDef {
        id: "web",
        label: "Web",
        default_connector: None,
        hint: "Public websites, for train routes and booking links, admission ticket prices and purchase links. Prefer official sites and the reference links.",
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
    "*search*",
    "*get_*",
    "*list*",
    "*read*",
    "*find*",
    "*fetch*",
    "*query*",
    "*download*",
    "*lookup*",
];

/// Built-in Claude Code tools Pricy never needs. Blocked explicitly, because
/// restricting the built-in set with `--tools` also hides the claude.ai connectors.
pub const BLOCKED_BUILTINS: &[&str] = &[
    "Bash",
    "Edit",
    "Write",
    "NotebookEdit",
    "WebFetch",
    "WebSearch",
    "Task",
    "Agent",
    "Skill",
    "Workflow",
    "SendMessage",
    "RemoteTrigger",
    "PushNotification",
    "Monitor",
    "ScheduleWakeup",
    "CronCreate",
    "CronDelete",
    "CronList",
    "EnterWorktree",
    "ExitWorktree",
    "DesignSync",
    "ShareOnboardingGuide",
    "ReportFindings",
    "TaskCreate",
    "TaskUpdate",
    "TaskStop",
    "TaskOutput",
    "TaskGet",
    "TaskList",
    "TodoWrite",
    "KillShell",
    "BashOutput",
    "ExitPlanMode",
    "EnterPlanMode",
    "AskUserQuestion",
];

const LOCAL_READ_TOOLS: &[&str] = &["Read", "Grep", "Glob"];
const WEB_TOOLS: &[&str] = &["WebSearch", "WebFetch"];

/// "claude.ai Google Drive" → "mcp__claude_ai_Google_Drive"
pub fn tool_prefix(connector: &str) -> String {
    let norm: String = connector
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("mcp__{norm}")
}

/// Folder (relative to the workspace) holding a local source's files.
pub fn local_rel_dir(source_id: &str) -> Option<&'static str> {
    match source_id {
        "whatsapp" => Some("sources/whatsapp"),
        "saved" => Some("sources/answers"),
        _ => None,
    }
}

pub fn whatsapp_dir(workspace: &Path) -> PathBuf {
    workspace.join("sources").join("whatsapp")
}

/// Visible file names in a local source's folder, sorted.
pub fn local_files(workspace: &Path, source_id: &str) -> Vec<String> {
    let Some(rel) = local_rel_dir(source_id) else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(workspace.join(rel))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
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

/// Simple `*` wildcard match, as used by the read patterns.
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
    }
    pattern.ends_with('*')
        || parts
            .last()
            .is_none_or(|l| l.is_empty() || text.ends_with(l))
}

pub fn is_read_tool(tool: &str) -> bool {
    let name = tool.rsplit("__").next().unwrap_or(tool);
    READ_PATTERNS.iter().any(|p| glob_match(p, name))
}

/// Connectors that aren't used this turn are removed entirely (server-level deny), and
/// a used connector's non-read tools are removed too: Claude never sees them, and every
/// turn stays small (each connector adds its tool definitions to the context).
fn deny_connectors(disallowed: &mut Vec<String>, connectors: &[Connector], used: &[String]) {
    for c in connectors {
        let prefix = tool_prefix(&c.name);
        if used.contains(&prefix) {
            disallowed.extend(c.tools.iter().filter(|t| !is_read_tool(t)).cloned());
        } else {
            disallowed.push(prefix);
        }
    }
}

/// For workflow steps (drafts, final answers): everything needed is already in the conversation.
pub fn policy_no_tools(connectors: &[Connector]) -> ToolPolicy {
    let mut disallowed: Vec<String> = BLOCKED_BUILTINS.iter().map(|s| s.to_string()).collect();
    disallowed.extend(LOCAL_READ_TOOLS.iter().map(|s| s.to_string()));
    disallowed.extend(WEB_TOOLS.iter().map(|s| s.to_string()));
    disallowed.push("ToolSearch".into());
    deny_connectors(&mut disallowed, connectors, &[]);
    ToolPolicy {
        allowed: Vec::new(),
        disallowed,
        prompt: "## This step\n\nThis is a workflow step, not a search: use only the conversation so far and do not call any tools.\n".into(),
    }
}

/// `local_files(id)` lists the files of a local source (WhatsApp, Saved answers).
/// `connectors` is every connector the CLI knows (from the last status check); empty = unknown.
pub fn policy(
    selected: &[Selected],
    connectors: &[Connector],
    local_files: impl Fn(&str) -> Vec<String>,
) -> ToolPolicy {
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
                    "  Tools: `{prefix}__*` (if they are not listed, load them with ToolSearch, e.g. query \"{}\"). Read-only: anything that writes or sends will be denied. If the connector needs sign-in, say so and that it is fixed in Claude Code with /mcp.\n",
                    s.def.label.to_lowercase()
                ));
            }
            (None, "web") => {
                allowed.extend(WEB_TOOLS.iter().map(|s| s.to_string()));
                disallowed.retain(|d| !WEB_TOOLS.contains(&d.as_str()));
                prompt.push_str("  Tools: WebSearch and WebFetch (read-only). Use them only for trains and admission tickets, and always give the booking / purchase link.\n");
            }
            (None, id) => {
                uses_local = true;
                let files = local_files(id);
                let dir = local_rel_dir(id).unwrap_or("sources");
                if files.is_empty() {
                    prompt.push_str(if id == "whatsapp" {
                        "  No WhatsApp export is attached yet: tell the user to attach one in Settings.\n"
                    } else {
                        "  No saved answers yet.\n"
                    });
                } else {
                    prompt.push_str(&format!(
                        "  Folder `{dir}/` ({} file{}, relative to the working directory). Use Grep with case-insensitive patterns (try place-name variants and abbreviations, e.g. \"FCO|Fiumicino\"), then Read around the matches for prices and dates. Quote the file or chat name, the date and the sender as the source.\n",
                        files.len(),
                        if files.len() == 1 { "" } else { "s" },
                    ));
                }
            }
        }
    }

    let used: Vec<String> = selected
        .iter()
        .filter_map(|s| s.connector.as_deref().map(tool_prefix))
        .collect();
    deny_connectors(&mut disallowed, connectors, &used);

    if uses_local {
        allowed.extend(LOCAL_READ_TOOLS.iter().map(|s| s.to_string()));
    } else {
        disallowed.extend(LOCAL_READ_TOOLS.iter().map(|s| s.to_string()));
    }

    prompt.push_str(
        "\nSearch only the sources listed above; any other tool will be denied. If a source fails (for example it needs re-authentication), say which one and continue with the others.\n",
    );

    ToolPolicy {
        allowed,
        disallowed,
        prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_normalises_connector_names() {
        assert_eq!(
            tool_prefix("claude.ai Google Drive"),
            "mcp__claude_ai_Google_Drive"
        );
        assert_eq!(
            tool_prefix("claude.ai TINT-staging"),
            "mcp__claude_ai_TINT-staging"
        );
    }

    #[test]
    fn unused_connectors_and_write_tools_are_removed() {
        let conns = vec![
            Connector {
                name: "claude.ai Google Drive".into(),
                status: "Connected".into(),
                connected: true,
                tools: vec![
                    "mcp__claude_ai_Google_Drive__search_files".into(),
                    "mcp__claude_ai_Google_Drive__create_file".into(),
                ],
            },
            Connector {
                name: "claude.ai Figma".into(),
                status: "Connected".into(),
                connected: true,
                tools: vec![],
            },
        ];
        let drive = Selected {
            def: get("drive").unwrap(),
            connector: Some("claude.ai Google Drive".into()),
        };
        let p = policy(&[drive], &conns, |_| Vec::new());
        assert!(p.disallowed.iter().any(|d| d == "mcp__claude_ai_Figma"));
        assert!(
            p.disallowed
                .iter()
                .any(|d| d == "mcp__claude_ai_Google_Drive__create_file")
        );
        assert!(
            !p.disallowed
                .iter()
                .any(|d| d == "mcp__claude_ai_Google_Drive__search_files")
        );
        assert!(
            !p.disallowed
                .iter()
                .any(|d| d == "mcp__claude_ai_Google_Drive")
        );

        let p = policy_no_tools(&conns);
        assert!(
            p.disallowed
                .iter()
                .any(|d| d == "mcp__claude_ai_Google_Drive")
        );
    }

    #[test]
    fn read_patterns() {
        assert!(is_read_tool("mcp__x__search_files"));
        assert!(is_read_tool("mcp__x__get_file_metadata"));
        assert!(is_read_tool("mcp__x__list_recent_files"));
        assert!(!is_read_tool("mcp__x__create_file"));
        assert!(!is_read_tool("mcp__x__send_message"));
    }

    #[test]
    fn web_tools_only_when_web_selected() {
        let web = Selected {
            def: get("web").unwrap(),
            connector: None,
        };
        let p = policy(&[web], &[], |_| Vec::new());
        assert!(p.allowed.iter().any(|a| a == "WebSearch"));
        assert!(!p.disallowed.iter().any(|d| d == "WebFetch"));
        assert!(p.disallowed.iter().any(|d| d == "Read"));

        let drive = Selected {
            def: get("drive").unwrap(),
            connector: Some("claude.ai Google Drive".into()),
        };
        let p = policy(&[drive], &[], |_| Vec::new());
        assert!(p.disallowed.iter().any(|d| d == "WebSearch"));
    }

    #[test]
    fn local_tools_only_when_whatsapp_selected() {
        let drive = Selected {
            def: get("drive").unwrap(),
            connector: Some("claude.ai Google Drive".into()),
        };
        let p = policy(&[drive], &[], |_| Vec::new());
        assert!(
            p.allowed
                .iter()
                .any(|a| a == "mcp__claude_ai_Google_Drive__*search*")
        );
        assert!(p.disallowed.iter().any(|d| d == "Read"));

        let wa = Selected {
            def: get("whatsapp").unwrap(),
            connector: None,
        };
        let p = policy(&[wa], &[], |_| vec!["chat.txt".into()]);
        assert!(p.allowed.iter().any(|a| a == "Grep"));
        assert!(!p.disallowed.iter().any(|d| d == "Grep"));
        assert!(p.prompt.contains("sources/whatsapp/"));
    }
}
