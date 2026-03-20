use crate::onboarding::OnboardingState;
use gclaw_core::AgentEvent;
use std::path::PathBuf;

/// Result of submitting user input.
#[derive(Debug, Clone)]
pub enum SubmitResult {
    /// No action needed (empty input or handled locally).
    None,
    /// Normal message to send to the agent.
    Message(String),
    /// Skill invocation: `/skill-name args`.
    SkillInvocation { name: String, args: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub sender: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: String,
    pub messages: Vec<ChatMessage>,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    Idle,
    Thinking,
    Acting(String),
}

pub struct App {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor_position: usize,
    pub scroll_offset: u16,
    pub mode: Mode,
    pub agent_state: AgentState,
    pub model_name: String,
    pub conversation_id: String,
    pub should_quit: bool,
    pub conversations: Vec<Conversation>,
    pub active_conversation: usize,
    pub show_sidebar: bool,
    pub streaming_thinking: String,
    pub streaming_content: String,
    pub is_thinking: bool,
    pub thinking_collapsed: bool,
    pub onboarding: Option<OnboardingState>,
    pub startup_warnings: Vec<String>,
    /// Known skill names for slash-command dispatch.
    pub skill_names: Vec<String>,
}

impl App {
    pub fn new(model_name: String, conversation_id: String) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor_position: 0,
            scroll_offset: 0,
            mode: Mode::Insert,
            agent_state: AgentState::Idle,
            model_name,
            conversation_id: conversation_id.clone(),
            should_quit: false,
            conversations: vec![Conversation {
                id: conversation_id,
                messages: Vec::new(),
                preview: String::new(),
            }],
            active_conversation: 0,
            show_sidebar: false,
            streaming_thinking: String::new(),
            streaming_content: String::new(),
            is_thinking: false,
            thinking_collapsed: false,
            onboarding: None,
            startup_warnings: Vec::new(),
            skill_names: Vec::new(),
        }
    }

    pub fn with_skill_names(mut self, names: Vec<String>) -> Self {
        self.skill_names = names;
        self
    }

    pub fn with_startup_warnings(mut self, warnings: Vec<String>) -> Self {
        // Add warnings as system messages so user sees them
        for w in &warnings {
            self.messages.push(ChatMessage {
                sender: "System".to_string(),
                content: w.clone(),
            });
        }
        self.startup_warnings = warnings;
        self
    }

    pub fn with_onboarding(mut self, workspace_dir: PathBuf) -> Self {
        self.onboarding = Some(OnboardingState::new(workspace_dir));
        self
    }

    pub fn is_onboarding(&self) -> bool {
        self.onboarding.is_some()
    }

    /// Returns a `SubmitResult` indicating what to do with the input.
    pub fn submit_input(&mut self) -> SubmitResult {
        if self.input.trim().is_empty() {
            return SubmitResult::None;
        }
        let input = self.input.clone();

        // Handle /model command locally
        if let Some(model_arg) = input.strip_prefix("/model") {
            let model_arg = model_arg.trim();
            self.input.clear();
            self.cursor_position = 0;
            if model_arg.is_empty() {
                self.messages.push(ChatMessage {
                    sender: "System".to_string(),
                    content: format!("Current model: {}", self.model_name),
                });
            } else {
                self.model_name = model_arg.to_string();
                self.messages.push(ChatMessage {
                    sender: "System".to_string(),
                    content: format!("Switched to model: {model_arg}"),
                });
            }
            return SubmitResult::None;
        }

        // Handle /skills command
        if input.trim() == "/skills" {
            self.input.clear();
            self.cursor_position = 0;
            if self.skill_names.is_empty() {
                self.messages.push(ChatMessage {
                    sender: "System".to_string(),
                    content: "No skills available.".to_string(),
                });
            } else {
                let list = self.skill_names.join(", ");
                self.messages.push(ChatMessage {
                    sender: "System".to_string(),
                    content: format!("Available skills: {list}"),
                });
            }
            return SubmitResult::None;
        }

        // Check for skill invocation: /skill-name args
        if let Some(without_slash) = input.strip_prefix('/') {
            let (cmd, args) = match without_slash.split_once(char::is_whitespace) {
                Some((c, a)) => (c, a.to_string()),
                None => (without_slash, String::new()),
            };
            if self.skill_names.iter().any(|s| s == cmd) {
                self.messages.push(ChatMessage {
                    sender: "You".to_string(),
                    content: input.clone(),
                });
                self.input.clear();
                self.cursor_position = 0;
                self.streaming_thinking.clear();
                self.streaming_content.clear();
                self.is_thinking = false;
                self.thinking_collapsed = false;
                self.agent_state = AgentState::Thinking;
                return SubmitResult::SkillInvocation {
                    name: cmd.to_string(),
                    args,
                };
            }
        }

        self.messages.push(ChatMessage {
            sender: "You".to_string(),
            content: input.clone(),
        });
        self.input.clear();
        self.cursor_position = 0;
        self.streaming_thinking.clear();
        self.streaming_content.clear();
        self.is_thinking = false;
        self.thinking_collapsed = false;
        self.agent_state = AgentState::Thinking;
        SubmitResult::Message(input)
    }

    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::ThinkStart => {
                self.is_thinking = true;
                self.agent_state = AgentState::Thinking;
            }
            AgentEvent::ThinkDelta(content) => {
                self.streaming_thinking.push_str(&content);
            }
            AgentEvent::ThinkEnd => {
                self.is_thinking = false;
            }
            AgentEvent::StreamDelta(content) => {
                self.streaming_content.push_str(&content);
            }
            AgentEvent::ToolCallStart { name, .. } => {
                self.agent_state = AgentState::Acting(name);
            }
            AgentEvent::ToolResult {
                content, is_error, ..
            } => {
                let prefix = if is_error { "[ERROR] " } else { "" };
                self.messages.push(ChatMessage {
                    sender: "Tool".to_string(),
                    content: format!("{prefix}{content}"),
                });
            }
            AgentEvent::Done(content) => {
                // Finalize thinking block into messages if present
                if !self.streaming_thinking.is_empty() {
                    self.messages.push(ChatMessage {
                        sender: "Thinking".to_string(),
                        content: self.streaming_thinking.clone(),
                    });
                    self.streaming_thinking.clear();
                }
                let display = if self.streaming_content.is_empty() {
                    content
                } else {
                    self.streaming_content.clone()
                };
                self.messages.push(ChatMessage {
                    sender: "Assistant".to_string(),
                    content: display,
                });
                self.streaming_content.clear();
                self.is_thinking = false;
                self.agent_state = AgentState::Idle;
            }
            AgentEvent::Error(err) => {
                self.messages.push(ChatMessage {
                    sender: "Error".to_string(),
                    content: err,
                });
                self.is_thinking = false;
                self.agent_state = AgentState::Idle;
            }
        }
    }

    pub fn toggle_thinking_collapsed(&mut self) {
        self.thinking_collapsed = !self.thinking_collapsed;
    }

    pub fn toggle_sidebar(&mut self) {
        self.show_sidebar = !self.show_sidebar;
    }

    pub fn new_conversation(&mut self) {
        // Save current messages into active conversation
        self.conversations[self.active_conversation].messages = self.messages.clone();
        if let Some(first_user_msg) = self.messages.iter().find(|m| m.sender == "You") {
            self.conversations[self.active_conversation].preview = first_user_msg.content.clone();
        }

        let new_id = format!("conv-{}", uuid::Uuid::new_v4());
        self.conversations.push(Conversation {
            id: new_id.clone(),
            messages: Vec::new(),
            preview: String::new(),
        });
        self.active_conversation = self.conversations.len() - 1;
        self.conversation_id = new_id;
        self.messages.clear();
        self.input.clear();
        self.cursor_position = 0;
        self.streaming_thinking.clear();
        self.streaming_content.clear();
        self.is_thinking = false;
        self.agent_state = AgentState::Idle;
    }

    pub fn switch_conversation(&mut self, index: usize) {
        if index >= self.conversations.len() || index == self.active_conversation {
            return;
        }

        // Save current state
        self.conversations[self.active_conversation].messages = self.messages.clone();
        if let Some(first_user_msg) = self.messages.iter().find(|m| m.sender == "You") {
            self.conversations[self.active_conversation].preview = first_user_msg.content.clone();
        }

        // Load target conversation
        self.active_conversation = index;
        self.messages = self.conversations[index].messages.clone();
        self.conversation_id = self.conversations[index].id.clone();
        self.input.clear();
        self.cursor_position = 0;
        self.streaming_thinking.clear();
        self.streaming_content.clear();
        self.is_thinking = false;
        self.agent_state = AgentState::Idle;
    }

    pub fn next_conversation(&mut self) {
        if self.active_conversation + 1 < self.conversations.len() {
            self.switch_conversation(self.active_conversation + 1);
        }
    }

    pub fn prev_conversation(&mut self) {
        if self.active_conversation > 0 {
            self.switch_conversation(self.active_conversation - 1);
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_position, c);
        self.cursor_position += c.len_utf8();
    }

    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 {
            let prev = self.input[..self.cursor_position]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_position -= prev;
            self.input.remove(self.cursor_position);
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            let prev = self.input[..self.cursor_position]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_position -= prev;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.input.len() {
            let next = self.input[self.cursor_position..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_position += next;
        }
    }
}
