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

    async fn list_conversations(&self, limit: usize) -> Result<Vec<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GclawError::Memory(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT conversation_id FROM messages
             GROUP BY conversation_id
             ORDER BY MAX(created_at) DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit], |row| {
            let id: String = row.get(0)?;
            Ok(id)
        })?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        Ok(ids)
    }

    async fn delete_conversation(&self, conversation_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GclawError::Memory(e.to_string()))?;
        conn.execute(
            "DELETE FROM messages WHERE conversation_id = ?1",
            rusqlite::params![conversation_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    #[tokio::test]
    async fn store_and_retrieve() {
        let mem = SqliteMemory::in_memory().unwrap();
        let msgs = vec![
            make_msg(Role::User, "hello"),
            make_msg(Role::Assistant, "hi there"),
        ];
        mem.store("convo1", &msgs).await.unwrap();

        let retrieved = mem.retrieve("convo1", 10).await.unwrap();
        assert_eq!(retrieved.len(), 2);
        assert_eq!(retrieved[0].content, "hello");
        assert_eq!(retrieved[0].role, Role::User);
        assert_eq!(retrieved[1].content, "hi there");
        assert_eq!(retrieved[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn retrieve_empty_conversation() {
        let mem = SqliteMemory::in_memory().unwrap();
        let retrieved = mem.retrieve("nonexistent", 10).await.unwrap();
        assert!(retrieved.is_empty());
    }

    #[tokio::test]
    async fn search_finds_matching_messages() {
        let mem = SqliteMemory::in_memory().unwrap();
        mem.store(
            "c1",
            &[
                make_msg(Role::User, "tell me about zebras"),
                make_msg(Role::Assistant, "they have stripes"),
                make_msg(Role::User, "what about python"),
            ],
        )
        .await
        .unwrap();

        let results = mem.search("zebra", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "tell me about zebras");
    }

    #[tokio::test]
    async fn retrieve_respects_limit() {
        let mem = SqliteMemory::in_memory().unwrap();
        let msgs: Vec<Message> = (0..5)
            .map(|i| make_msg(Role::User, &format!("msg {i}")))
            .collect();
        mem.store("c1", &msgs).await.unwrap();

        let retrieved = mem.retrieve("c1", 3).await.unwrap();
        assert_eq!(retrieved.len(), 3);
    }

    #[tokio::test]
    async fn list_conversations() {
        let mem = SqliteMemory::in_memory().unwrap();
        mem.store("convo1", &[make_msg(Role::User, "hello")])
            .await
            .unwrap();
        mem.store("convo2", &[make_msg(Role::User, "world")])
            .await
            .unwrap();

        let convos = mem.list_conversations(10).await.unwrap();
        assert_eq!(convos.len(), 2);
        // Both conversations should be listed (order may vary within same timestamp)
        assert!(convos.contains(&"convo1".to_string()));
        assert!(convos.contains(&"convo2".to_string()));
    }

    #[tokio::test]
    async fn delete_conversation() {
        let mem = SqliteMemory::in_memory().unwrap();
        mem.store("convo1", &[make_msg(Role::User, "hello")])
            .await
            .unwrap();
        mem.delete_conversation("convo1").await.unwrap();
        let retrieved = mem.retrieve("convo1", 10).await.unwrap();
        assert!(retrieved.is_empty());
    }

    #[tokio::test]
    async fn retrieve_ordering_is_chronological() {
        let mem = SqliteMemory::in_memory().unwrap();
        mem.store("c1", &[make_msg(Role::User, "first")])
            .await
            .unwrap();
        mem.store("c1", &[make_msg(Role::User, "second")])
            .await
            .unwrap();

        let retrieved = mem.retrieve("c1", 10).await.unwrap();
        assert_eq!(retrieved[0].content, "first");
        assert_eq!(retrieved[1].content, "second");
    }
}
