use crate::app::App;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let input = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Input (Enter to send, Esc to quit) "),
        );
    f.render_widget(input, area);

    // Set cursor position
    let x = area.x + app.cursor_position as u16 + 1;
    let y = area.y + 1;
    f.set_cursor_position((x, y));
}
