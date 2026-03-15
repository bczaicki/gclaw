use crate::app::App;
use crate::widgets;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App) {
    if let Some(ref onboarding) = app.onboarding {
        widgets::onboarding::render(f, onboarding);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // chat
            Constraint::Length(3), // input
            Constraint::Length(3), // status
        ])
        .split(f.area());

    widgets::chat::render(f, app, chunks[0]);
    widgets::input::render(f, app, chunks[1]);
    widgets::status::render(f, app, chunks[2]);
}
