use crate::onboarding::OnboardingState;
use gclaw_core::AgentEvent;
use std::path::PathBuf;

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
    pub streaming_thinking: String,
    pub streaming_content: String,
    pub is_thinking: bool,
    pub thinking_collapsed: bool,
    pub onboarding: Option<OnboardingState>,
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
            conversation_id,
            should_quit: false,
            streaming_thinking: String::new(),
            streaming_content: String::new(),
            is_thinking: false,
            thinking_collapsed: false,
            onboarding: None,
        }
    }

    pub fn with_onboarding(mut self, workspace_dir: PathBuf) -> Self {
        self.onboarding = Some(OnboardingState::new(workspace_dir));
        self
    }

    pub fn is_onboarding(&self) -> bool {
        self.onboarding.is_some()
    }

    pub fn submit_input(&mut self) -> Option<String> {
        if self.input.trim().is_empty() {
            return None;
        }
        let input = self.input.clone();
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
        Some(input)
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
