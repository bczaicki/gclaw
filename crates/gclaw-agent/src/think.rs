use gclaw_core::types::AgentEvent;
use tokio::sync::mpsc;

/// Parses a stream of text chunks, detecting `<think>...</think>` tags
/// and emitting separate ThinkDelta / StreamDelta events.
///
/// Handles tags split across chunk boundaries by buffering potential
/// partial tags.
pub struct ThinkParser {
    in_think: bool,
    buffer: String,
    content_accum: String,
    think_accum: String,
}

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

impl ThinkParser {
    pub fn new() -> Self {
        Self {
            in_think: false,
            buffer: String::new(),
            content_accum: String::new(),
            think_accum: String::new(),
        }
    }

    /// Feed a chunk of text and emit events.
    pub fn feed(&mut self, chunk: &str, tx: &mpsc::UnboundedSender<AgentEvent>) {
        self.buffer.push_str(chunk);

        loop {
            if self.in_think {
                // Look for </think>
                if let Some(pos) = self.buffer.find(THINK_CLOSE) {
                    let thinking = self.buffer[..pos].to_string();
                    if !thinking.is_empty() {
                        self.think_accum.push_str(&thinking);
                        let _ = tx.send(AgentEvent::ThinkDelta(thinking));
                    }
                    self.buffer = self.buffer[pos + THINK_CLOSE.len()..].to_string();
                    self.in_think = false;
                    let _ = tx.send(AgentEvent::ThinkEnd);
                } else if self.buffer.len() > THINK_CLOSE.len() {
                    // Emit everything except the last few chars that could be
                    // a partial </think> tag
                    let safe_len = self.buffer.len() - THINK_CLOSE.len() + 1;
                    let safe = self.buffer[..safe_len].to_string();
                    if !safe.is_empty() {
                        self.think_accum.push_str(&safe);
                        let _ = tx.send(AgentEvent::ThinkDelta(safe));
                    }
                    self.buffer = self.buffer[safe_len..].to_string();
                    break;
                } else {
                    // Buffer too short to tell, wait for more
                    break;
                }
            } else {
                // Look for <think>
                if let Some(pos) = self.buffer.find(THINK_OPEN) {
                    // Emit content before the tag
                    let before = self.buffer[..pos].to_string();
                    if !before.is_empty() {
                        self.content_accum.push_str(&before);
                        let _ = tx.send(AgentEvent::StreamDelta(before));
                    }
                    self.buffer = self.buffer[pos + THINK_OPEN.len()..].to_string();
                    self.in_think = true;
                    let _ = tx.send(AgentEvent::ThinkStart);
                } else if self.buffer.len() > THINK_OPEN.len() {
                    // Emit safe content
                    let safe_len = self.buffer.len() - THINK_OPEN.len() + 1;
                    let safe = self.buffer[..safe_len].to_string();
                    if !safe.is_empty() {
                        self.content_accum.push_str(&safe);
                        let _ = tx.send(AgentEvent::StreamDelta(safe));
                    }
                    self.buffer = self.buffer[safe_len..].to_string();
                    break;
                } else {
                    break;
                }
            }
        }
    }

    /// Flush any remaining buffer content at end of stream.
    pub fn flush(&mut self, tx: &mpsc::UnboundedSender<AgentEvent>) {
        if !self.buffer.is_empty() {
            if self.in_think {
                self.think_accum.push_str(&self.buffer);
                let _ = tx.send(AgentEvent::ThinkDelta(self.buffer.clone()));
                let _ = tx.send(AgentEvent::ThinkEnd);
            } else {
                self.content_accum.push_str(&self.buffer);
                let _ = tx.send(AgentEvent::StreamDelta(self.buffer.clone()));
            }
            self.buffer.clear();
        }
    }

    /// The accumulated content (non-thinking) text.
    pub fn content(&self) -> &str {
        &self.content_accum
    }

    /// The accumulated thinking text.
    pub fn thinking(&self) -> &str {
        &self.think_accum
    }
}

impl Default for ThinkParser {
    fn default() -> Self {
        Self::new()
    }
}
