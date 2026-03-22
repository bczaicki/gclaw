use crate::app::{AgentState, App};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

const THINK_COLOR: Color = Color::Indexed(243); // medium gray
const THINK_BORDER: Color = Color::Indexed(238); // dark gray
const THINK_LABEL_COLOR: Color = Color::Indexed(140); // muted purple
const DIM: Color = Color::Indexed(245);

pub fn render(f: &mut Frame, app: &mut App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        match msg.sender.as_str() {
            "Thinking" => {
                render_thinking_block(&msg.content, &mut lines, false);
                lines.push(Line::from(""));
            }
            "Debug" => {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        &msg.content,
                        Style::default()
                            .fg(DIM)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            _ => {
                let (color, prefix) = match msg.sender.as_str() {
                    "You" => (Color::White, "● "),
                    "Assistant" => (Color::Green, "● "),
                    "Tool" => (Color::Yellow, "  ├ "),
                    "Error" => (Color::Red, "● "),
                    "System" => (Color::DarkGray, "  "),
                    _ => (Color::White, "● "),
                };

                let content_lines: Vec<&str> = msg.content.lines().collect();
                if let Some((first, rest)) = content_lines.split_first() {
                    // First line: prefix + content inline
                    lines.push(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(color)),
                        Span::styled(
                            first.to_string(),
                            Style::default().fg(color),
                        ),
                    ]));
                    // Continuation lines indented
                    let indent = " ".repeat(prefix.len());
                    for line in rest {
                        lines.push(Line::from(Span::styled(
                            format!("{indent}{line}"),
                            Style::default().fg(color),
                        )));
                    }
                }
                lines.push(Line::from(""));
            }
        }
    }

    // Show live streaming thinking
    if !app.streaming_thinking.is_empty() {
        render_thinking_block(&app.streaming_thinking, &mut lines, app.is_thinking);
        if !app.is_thinking {
            lines.push(Line::from(""));
        }
    }

    // Show live streaming content
    if !app.streaming_content.is_empty() {
        let content_lines: Vec<&str> = app.streaming_content.lines().collect();
        if let Some((first, rest)) = content_lines.split_first() {
            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(Color::Green)),
                Span::styled(
                    first.to_string(),
                    Style::default().fg(Color::Green),
                ),
            ]));
            for line in rest {
                lines.push(Line::from(Span::styled(
                    format!("  {line}"),
                    Style::default().fg(Color::Green),
                )));
            }
        }
    }

    // Cursor indicator when actively streaming
    let is_streaming = app.is_thinking
        || (!app.streaming_content.is_empty() && matches!(app.agent_state, AgentState::Thinking));
    if is_streaming {
        let cursor_color = if app.is_thinking {
            THINK_LABEL_COLOR
        } else {
            Color::Green
        };
        lines.push(Line::from(Span::styled(
            "  ▌",
            Style::default().fg(cursor_color),
        )));
    }

    // Waiting indicator when no content has arrived yet
    if app.streaming_thinking.is_empty()
        && app.streaming_content.is_empty()
        && matches!(app.agent_state, AgentState::Thinking)
    {
        lines.push(Line::from(vec![Span::styled(
            "  Waiting for response...",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )]));
    }

    // Auto-scroll: compute offset to keep bottom visible
    let inner_height = area.height.saturating_sub(2) as usize; // -2 for borders
    if app.auto_scroll && lines.len() > inner_height {
        app.scroll_offset = (lines.len() - inner_height) as u16;
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Chat "))
        .wrap(Wrap { trim: false })
        .scroll((app.scroll_offset, 0));

    f.render_widget(paragraph, area);
}

/// Render a thinking block with a distinct visual style:
/// dimmed text, left border indicator, italic label.
fn render_thinking_block(content: &str, lines: &mut Vec<Line>, is_active: bool) {
    // Header
    let label = if is_active { "thinking..." } else { "thought" };
    lines.push(Line::from(vec![
        Span::styled("  ┌─ ", Style::default().fg(THINK_BORDER)),
        Span::styled(
            label,
            Style::default()
                .fg(THINK_LABEL_COLOR)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::styled(" ─".to_string(), Style::default().fg(THINK_BORDER)),
    ]));

    // Content lines with left border
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            lines.push(Line::from(Span::styled(
                "  │",
                Style::default().fg(THINK_BORDER),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  │ ", Style::default().fg(THINK_BORDER)),
                Span::styled(
                    trimmed.to_string(),
                    Style::default()
                        .fg(THINK_COLOR)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }

    // Footer
    if !is_active {
        lines.push(Line::from(Span::styled(
            "  └───",
            Style::default().fg(THINK_BORDER),
        )));
    }
}
