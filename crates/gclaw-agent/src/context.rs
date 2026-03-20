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
