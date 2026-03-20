use gclaw_core::types::{Message, Role, ToolDefinition};

pub struct ConversationContext {
    system_prompt: String,
    messages: Vec<Message>,
    tool_definitions: Vec<ToolDefinition>,
}

impl ConversationContext {
    pub fn new(system_prompt: String) -> Self {
        Self {
            system_prompt,
            messages: Vec::new(),
            tool_definitions: Vec::new(),
        }
    }

    pub fn set_tools(&mut self, tools: Vec<ToolDefinition>) {
        self.tool_definitions = tools;
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }

    pub fn messages_with_system(&self) -> Vec<Message> {
        let mut msgs = vec![Message {
            role: Role::System,
            content: self.system_prompt.clone(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        msgs.extend(self.messages.clone());
        msgs
    }

    pub fn tool_definitions(&self) -> &[ToolDefinition] {
        &self.tool_definitions
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn load_history(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// Estimate the total token count (rough: 4 chars ≈ 1 token).
    pub fn estimated_tokens(&self) -> usize {
        let system_tokens = self.system_prompt.len() / 4;
        let msg_tokens: usize = self
            .messages
            .iter()
            .map(|m| m.content.len() / 4 + 10) // 10 for role overhead
            .sum();
        system_tokens + msg_tokens
    }

    /// If context exceeds `max_tokens`, compress older messages into a summary.
    /// Keeps the most recent `keep_recent` messages intact.
    pub fn compress_if_needed(&mut self, max_tokens: usize, keep_recent: usize) {
        if self.estimated_tokens() <= max_tokens || self.messages.len() <= keep_recent {
            return;
        }

        let split_at = self.messages.len().saturating_sub(keep_recent);
        let old_messages: Vec<_> = self.messages.drain(..split_at).collect();

        // Build a summary of the removed messages
        let summary = summarize_messages(&old_messages);

        // Insert the summary as the first message
        self.messages.insert(
            0,
            Message {
                role: Role::System,
                content: format!("[Conversation summary: {summary}]"),
                tool_calls: vec![],
                tool_call_id: None,
            },
        );
    }
}

/// Create a brief summary of a set of messages for context compression.
fn summarize_messages(messages: &[Message]) -> String {
    let mut topics = Vec::new();
    let mut tool_names = Vec::new();

    for msg in messages {
        match msg.role {
            Role::User => {
                // Take the first 100 chars as a topic snippet
                let snippet: String = msg.content.chars().take(100).collect();
                topics.push(snippet);
            }
            Role::Tool => {
                if let Some(ref id) = msg.tool_call_id {
                    if !tool_names.contains(id) {
                        tool_names.push(id.clone());
                    }
                }
            }
            _ => {}
        }
    }

    let mut parts = Vec::new();
    if !topics.is_empty() {
        let topic_str = if topics.len() <= 3 {
            topics.join("; ")
        } else {
            format!(
                "{}; ... and {} more exchanges",
                topics[..3].join("; "),
                topics.len() - 3
            )
        };
        parts.push(format!("Topics discussed: {topic_str}"));
    }
    if !tool_names.is_empty() {
        parts.push(format!("Tools used: {}", tool_names.len()));
    }
    parts.push(format!("{} messages compressed", messages.len()));

    parts.join(". ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gclaw_core::types::{Message, Role, ToolDefinition};

    fn user_msg(content: &str) -> Message {
        Message {
            role: Role::User,
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    fn assistant_msg(content: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    #[test]
    fn messages_with_system_prepends_system_prompt() {
        let ctx = ConversationContext::new("You are helpful.".to_string());
        let msgs = ctx.messages_with_system();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::System);
        assert_eq!(msgs[0].content, "You are helpful.");
    }

    #[test]
    fn messages_with_system_includes_added_messages() {
        let mut ctx = ConversationContext::new("system".to_string());
        ctx.add_message(user_msg("hi"));
        ctx.add_message(assistant_msg("hello"));

        let msgs = ctx.messages_with_system();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, Role::System);
        assert_eq!(msgs[1].role, Role::User);
        assert_eq!(msgs[1].content, "hi");
        assert_eq!(msgs[2].role, Role::Assistant);
        assert_eq!(msgs[2].content, "hello");
    }

    #[test]
    fn load_history_replaces_messages() {
        let mut ctx = ConversationContext::new("sys".to_string());
        ctx.add_message(user_msg("old"));
        assert_eq!(ctx.messages().len(), 1);

        let history = vec![user_msg("new1"), assistant_msg("new2")];
        ctx.load_history(history);

        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].content, "new1");
        assert_eq!(ctx.messages()[1].content, "new2");
    }

    #[test]
    fn add_message_appends() {
        let mut ctx = ConversationContext::new("sys".to_string());
        ctx.add_message(user_msg("first"));
        ctx.add_message(user_msg("second"));
        assert_eq!(ctx.messages().len(), 2);
        assert_eq!(ctx.messages()[0].content, "first");
        assert_eq!(ctx.messages()[1].content, "second");
    }

    #[test]
    fn estimated_tokens_rough_count() {
        let mut ctx = ConversationContext::new("system prompt".to_string());
        ctx.add_message(user_msg("Hello world")); // ~13 chars / 4 + 10 = ~13
        assert!(ctx.estimated_tokens() > 0);
    }

    #[test]
    fn compress_reduces_messages() {
        let mut ctx = ConversationContext::new("sys".to_string());
        for i in 0..20 {
            ctx.add_message(user_msg(&format!("message {i} with some content")));
            ctx.add_message(assistant_msg(&format!("response {i}")));
        }
        assert_eq!(ctx.messages().len(), 40);

        // Compress with a very low token limit to force compression
        ctx.compress_if_needed(100, 6);

        // Should have 6 recent messages + 1 summary
        assert_eq!(ctx.messages().len(), 7);
        assert!(ctx.messages()[0].content.contains("[Conversation summary:"));
    }

    #[test]
    fn compress_noop_when_under_limit() {
        let mut ctx = ConversationContext::new("sys".to_string());
        ctx.add_message(user_msg("hi"));
        ctx.add_message(assistant_msg("hello"));

        ctx.compress_if_needed(100_000, 10);
        // Should not compress — only 2 messages
        assert_eq!(ctx.messages().len(), 2);
    }

    #[test]
    fn set_tools_and_retrieve() {
        let mut ctx = ConversationContext::new("sys".to_string());
        assert!(ctx.tool_definitions().is_empty());

        let tools = vec![ToolDefinition {
            name: "search".to_string(),
            description: "Search the web".to_string(),
            parameters: serde_json::json!({}),
        }];
        ctx.set_tools(tools);

        assert_eq!(ctx.tool_definitions().len(), 1);
        assert_eq!(ctx.tool_definitions()[0].name, "search");
    }
}
