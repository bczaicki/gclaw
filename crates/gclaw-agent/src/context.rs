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
