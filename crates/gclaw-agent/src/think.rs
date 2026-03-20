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

/// Find the largest byte index <= `target` that is a char boundary in `s`.
fn floor_char_boundary(s: &str, target: usize) -> usize {
    if target >= s.len() {
        return s.len();
    }
    let mut i = target;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

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
                    // a partial </think> tag. Floor to a char boundary to avoid
                    // splitting multi-byte characters.
                    let raw = self.buffer.len() - THINK_CLOSE.len() + 1;
                    let safe_len = floor_char_boundary(&self.buffer, raw);
                    if safe_len == 0 {
                        break;
                    }
                    let safe = self.buffer[..safe_len].to_string();
                    self.think_accum.push_str(&safe);
                    let _ = tx.send(AgentEvent::ThinkDelta(safe));
                    self.buffer = self.buffer[safe_len..].to_string();
                    break;
                } else {
                    break;
                }
            } else {
                // Look for <think>
                if let Some(pos) = self.buffer.find(THINK_OPEN) {
                    let before = self.buffer[..pos].to_string();
                    if !before.is_empty() {
                        self.content_accum.push_str(&before);
                        let _ = tx.send(AgentEvent::StreamDelta(before));
                    }
                    self.buffer = self.buffer[pos + THINK_OPEN.len()..].to_string();
                    self.in_think = true;
                    let _ = tx.send(AgentEvent::ThinkStart);
                } else if self.buffer.len() > THINK_OPEN.len() {
                    // Emit safe content, floored to a char boundary.
                    let raw = self.buffer.len() - THINK_OPEN.len() + 1;
                    let safe_len = floor_char_boundary(&self.buffer, raw);
                    if safe_len == 0 {
                        break;
                    }
                    let safe = self.buffer[..safe_len].to_string();
                    self.content_accum.push_str(&safe);
                    let _ = tx.send(AgentEvent::StreamDelta(safe));
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

#[cfg(test)]
mod tests {
    use super::*;
    use gclaw_core::types::AgentEvent;
    use tokio::sync::mpsc;

    /// Collects all events currently in the receiver into a Vec.
    fn collect_events(rx: &mut mpsc::UnboundedReceiver<AgentEvent>) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        events
    }

    /// Helper to match StreamDelta content.
    fn stream_delta_text(events: &[AgentEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::StreamDelta(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Helper to match ThinkDelta content.
    fn think_delta_text(events: &[AgentEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ThinkDelta(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Helper: true if the event list contains a ThinkStart.
    fn has_think_start(events: &[AgentEvent]) -> bool {
        events.iter().any(|e| matches!(e, AgentEvent::ThinkStart))
    }

    /// Helper: true if the event list contains a ThinkEnd.
    fn has_think_end(events: &[AgentEvent]) -> bool {
        events.iter().any(|e| matches!(e, AgentEvent::ThinkEnd))
    }

    // ---------------------------------------------------------------
    // Basic text without think tags passes through as StreamDelta
    // ---------------------------------------------------------------
    #[test]
    fn plain_text_emits_stream_delta() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("Hello, world!", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        let text = stream_delta_text(&events);
        assert_eq!(text, "Hello, world!");
        assert!(!has_think_start(&events));
    }

    // ---------------------------------------------------------------
    // <think>...</think> tags emit ThinkStart, ThinkDelta, ThinkEnd
    // ---------------------------------------------------------------
    #[test]
    fn think_tags_emit_think_events() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("<think>reasoning here</think>answer", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert!(has_think_start(&events));
        assert!(has_think_end(&events));
        assert_eq!(think_delta_text(&events), "reasoning here");
        assert_eq!(stream_delta_text(&events), "answer");
    }

    // ---------------------------------------------------------------
    // Tags split across multiple feed() calls
    // ---------------------------------------------------------------
    #[test]
    fn split_open_tag_across_feeds() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("<thi", &tx);
        parser.feed("nk>inner</think>after", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert!(has_think_start(&events));
        assert!(has_think_end(&events));
        assert_eq!(think_delta_text(&events), "inner");
        assert_eq!(stream_delta_text(&events), "after");
    }

    #[test]
    fn split_close_tag_across_feeds() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("<think>stuff</th", &tx);
        parser.feed("ink>tail", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert!(has_think_start(&events));
        assert!(has_think_end(&events));
        assert_eq!(think_delta_text(&events), "stuff");
        assert_eq!(stream_delta_text(&events), "tail");
    }

    #[test]
    fn tag_split_at_every_byte() {
        // Feed the entire input one byte-at-a-time to stress partial matching.
        let input = "before<think>thinking</think>after";
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        for ch in input.chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            parser.feed(s, &tx);
        }
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert!(has_think_start(&events));
        assert!(has_think_end(&events));
        assert_eq!(think_delta_text(&events), "thinking");
        assert_eq!(stream_delta_text(&events), "beforeafter");
    }

    // ---------------------------------------------------------------
    // Multi-byte characters (emoji, CJK) don't panic
    // ---------------------------------------------------------------
    #[test]
    fn multibyte_emoji_no_panic() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        // Feed emoji that are 4 bytes each, mixed with ASCII.
        parser.feed("Hello 🖥️ world 🚀!", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        let text = stream_delta_text(&events);
        assert_eq!(text, "Hello 🖥️ world 🚀!");
    }

    #[test]
    fn multibyte_chinese_no_panic() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("你好世界", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        let text = stream_delta_text(&events);
        assert_eq!(text, "你好世界");
    }

    #[test]
    fn emoji_inside_think_block() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("<think>🖥️🚀 reasoning 你好</think>done", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert_eq!(think_delta_text(&events), "🖥️🚀 reasoning 你好");
        assert_eq!(stream_delta_text(&events), "done");
    }

    #[test]
    fn emoji_split_across_feeds() {
        // Feed emoji bytes one at a time — must not panic on non-char-boundaries.
        let input = "🖥️<think>🚀</think>你好";
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        for ch in input.chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            parser.feed(s, &tx);
        }
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert!(has_think_start(&events));
        assert!(has_think_end(&events));
        assert_eq!(think_delta_text(&events), "🚀");
        assert_eq!(stream_delta_text(&events), "🖥️你好");
    }

    // ---------------------------------------------------------------
    // Nested content after think block
    // ---------------------------------------------------------------
    #[test]
    fn content_before_and_after_think() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("before<think>middle</think>after", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert_eq!(stream_delta_text(&events), "beforeafter");
        assert_eq!(think_delta_text(&events), "middle");
    }

    #[test]
    fn multiple_think_blocks() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("a<think>t1</think>b<think>t2</think>c", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert_eq!(stream_delta_text(&events), "abc");
        assert_eq!(think_delta_text(&events), "t1t2");
    }

    // ---------------------------------------------------------------
    // Empty think blocks
    // ---------------------------------------------------------------
    #[test]
    fn empty_think_block() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("<think></think>output", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert!(has_think_start(&events));
        assert!(has_think_end(&events));
        assert_eq!(think_delta_text(&events), "");
        assert_eq!(stream_delta_text(&events), "output");
    }

    // ---------------------------------------------------------------
    // Flush with remaining content
    // ---------------------------------------------------------------
    #[test]
    fn flush_emits_remaining_buffer() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        // Feed a short string that stays entirely buffered (shorter than tag len).
        parser.feed("hi", &tx);
        // Nothing emitted yet because "hi" could be a prefix of "<think>".
        let partial = collect_events(&mut rx);
        // "hi" is only 2 chars, shorter than "<think>" (7 chars), so it's buffered.
        assert!(stream_delta_text(&partial).is_empty() || stream_delta_text(&partial) == "hi");

        parser.flush(&tx);
        let events = collect_events(&mut rx);
        let all_text: String = stream_delta_text(&partial) + &stream_delta_text(&events);
        assert_eq!(all_text, "hi");
    }

    #[test]
    fn flush_inside_think_emits_think_end() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        // Unclosed think tag — flush should close it.
        parser.feed("<think>unclosed", &tx);
        parser.flush(&tx);

        let events = collect_events(&mut rx);
        assert!(has_think_start(&events));
        assert!(has_think_end(&events));
        assert_eq!(think_delta_text(&events), "unclosed");
    }

    // ---------------------------------------------------------------
    // Accumulators track content correctly
    // ---------------------------------------------------------------
    #[test]
    fn accumulators_correct() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut parser = ThinkParser::new();

        parser.feed("hello<think>brain</think>world", &tx);
        parser.flush(&tx);

        assert_eq!(parser.content(), "helloworld");
        assert_eq!(parser.thinking(), "brain");
    }
}
