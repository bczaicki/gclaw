use crate::traits::Memory;
use crate::types::{Message, Role};
use crate::{GclawError, Result};
use async_trait::async_trait;
use rusqlite::Connection;
use std::sync::Mutex;

pub struct SqliteMemory {
    conn: Mutex<Connection>,
}

impl SqliteMemory {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls TEXT NOT NULL DEFAULT '[]',
                tool_call_id TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE INDEX IF NOT EXISTS idx_messages_convo
                ON messages(conversation_id, created_at);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn in_memory() -> Result<Self> {
        Self::new(":memory:")
    }
}

#[async_trait]
impl Memory for SqliteMemory {
    async fn store(&self, conversation_id: &str, messages: &[Message]) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GclawError::Memory(e.to_string()))?;
        for msg in messages {
            let role = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            let tool_calls = serde_json::to_string(&msg.tool_calls)?;
            conn.execute(
                "INSERT INTO messages (conversation_id, role, content, tool_calls, tool_call_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    conversation_id,
                    role,
                    msg.content,
                    tool_calls,
                    msg.tool_call_id
                ],
            )?;
        }
        Ok(())
    }

    async fn retrieve(&self, conversation_id: &str, limit: usize) -> Result<Vec<Message>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GclawError::Memory(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT role, content, tool_calls, tool_call_id FROM messages
             WHERE conversation_id = ?1
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![conversation_id, limit], |row| {
            let role_str: String = row.get(0)?;
            let content: String = row.get(1)?;
            let tool_calls_str: String = row.get(2)?;
            let tool_call_id: Option<String> = row.get(3)?;
            Ok((role_str, content, tool_calls_str, tool_call_id))
        })?;

        let mut messages = Vec::new();
        for row in rows {
            let (role_str, content, tool_calls_str, tool_call_id) = row?;
            let role = match role_str.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let tool_calls = serde_json::from_str(&tool_calls_str).unwrap_or_default();
            messages.push(Message {
                role,
                content,
                tool_calls,
                tool_call_id,
            });
        }
        messages.reverse();
        Ok(messages)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Message>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GclawError::Memory(e.to_string()))?;
        let pattern = format!("%{query}%");
        let mut stmt = conn.prepare(
            "SELECT role, content, tool_calls, tool_call_id FROM messages
             WHERE content LIKE ?1
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![pattern, limit], |row| {
            let role_str: String = row.get(0)?;
            let content: String = row.get(1)?;
            let tool_calls_str: String = row.get(2)?;
            let tool_call_id: Option<String> = row.get(3)?;
            Ok((role_str, content, tool_calls_str, tool_call_id))
        })?;

        let mut messages = Vec::new();
        for row in rows {
            let (role_str, content, tool_calls_str, tool_call_id) = row?;
            let role = match role_str.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let tool_calls = serde_json::from_str(&tool_calls_str).unwrap_or_default();
            messages.push(Message {
                role,
                content,
                tool_calls,
                tool_call_id,
            });
        }
        Ok(messages)
    }
}
