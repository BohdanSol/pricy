//! Local SQLite storage: chat history and app settings.
//! Lives in the OS app-data dir (e.g. ~/Library/Application Support/com.coax.pricy/pricy.db).

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub claude_session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: i64,
    pub chat_id: String,
    /// "user" | "assistant" | "error"
    pub role: String,
    pub content: String,
    pub created_at: i64,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS chats (
                 id                TEXT PRIMARY KEY,
                 title             TEXT NOT NULL,
                 claude_session_id TEXT,
                 created_at        INTEGER NOT NULL,
                 updated_at        INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 chat_id    TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
                 role       TEXT NOT NULL,
                 content    TEXT NOT NULL,
                 created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_messages_chat ON messages(chat_id, id);
             CREATE TABLE IF NOT EXISTS settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        Ok(Self { conn })
    }

    // ---- chats ----

    pub fn list_chats(&self) -> Result<Vec<Chat>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, claude_session_id, created_at, updated_at
             FROM chats ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_chat)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_chat(&self, id: &str) -> Result<Option<Chat>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, title, claude_session_id, created_at, updated_at
                 FROM chats WHERE id = ?1",
                params![id],
                row_to_chat,
            )
            .optional()?)
    }

    pub fn create_chat(&self, title: &str) -> Result<Chat> {
        let ts = now_ms();
        let chat = Chat {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            claude_session_id: None,
            created_at: ts,
            updated_at: ts,
        };
        self.conn.execute(
            "INSERT INTO chats (id, title, claude_session_id, created_at, updated_at)
             VALUES (?1, ?2, NULL, ?3, ?3)",
            params![chat.id, chat.title, ts],
        )?;
        Ok(chat)
    }

    pub fn rename_chat(&self, id: &str, title: &str) -> Result<()> {
        self.conn
            .execute("UPDATE chats SET title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    pub fn delete_chat(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM chats WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn set_session_id(&self, chat_id: &str, session_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE chats SET claude_session_id = ?2 WHERE id = ?1",
            params![chat_id, session_id],
        )?;
        Ok(())
    }

    // ---- messages ----

    pub fn list_messages(&self, chat_id: &str) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chat_id, role, content, created_at
             FROM messages WHERE chat_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![chat_id], |r| {
            Ok(Message {
                id: r.get(0)?,
                chat_id: r.get(1)?,
                role: r.get(2)?,
                content: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_message(&self, chat_id: &str, role: &str, content: &str) -> Result<Message> {
        let ts = now_ms();
        self.conn.execute(
            "INSERT INTO messages (chat_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![chat_id, role, content, ts],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute(
            "UPDATE chats SET updated_at = ?2 WHERE id = ?1",
            params![chat_id, ts],
        )?;
        Ok(Message {
            id,
            chat_id: chat_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: ts,
        })
    }

    // ---- settings ----

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        if value.is_empty() {
            self.conn
                .execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        } else {
            self.conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
        Ok(())
    }
}

fn row_to_chat(r: &rusqlite::Row) -> rusqlite::Result<Chat> {
    Ok(Chat {
        id: r.get(0)?,
        title: r.get(1)?,
        claude_session_id: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
    })
}
