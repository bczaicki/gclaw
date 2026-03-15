use crate::app::{AgentState, App};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let state_str = if app.is_thinking {
        Span::styled(
            "Thinking...",
            Style::default()
                .fg(Color::Indexed(140))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        match &app.agent_state {
            AgentState::Idle => Span::styled("Idle", Style::default().fg(Color::Gray)),
            AgentState::Thinking => Span::styled(
                "Responding...",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            AgentState::Acting(name) => Span::styled(
                format!("Running: {name}"),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        }
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", app.model_name),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("│ "),
        Span::styled(
            format!("{} ", app.conversation_id),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("│ "),
        state_str,
    ]);

    let status =
        Paragraph::new(line).block(Block::default().borders(Borders::ALL).title(" Status "));
    f.render_widget(status, area);
}
