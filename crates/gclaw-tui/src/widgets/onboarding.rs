use crate::onboarding::{OnboardingState, OnboardingStep};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

const LOGO: &[&str] = &[
    r"            ___  ___ _       __",
    r"     __ _  / __\/ _ \ |     /__\ __      __",
    r"    / _` |/ /  / /_)/  \   / \/// \ /\ / /",
    r"   | (_| / /__/ ___/ /\ \ / _  \ \ V  V / ",
    r"    \__, \____\/   \/  \/ \/ \_/  \_/\_/  ",
    r"    |___/                                  ",
];

const TAGLINE: &str = "local-first ai agent gateway";

const WELCOME_LINES: &[&str] = &[
    "",
    "Welcome. Let's get you set up.",
    "",
    "This takes about 30 seconds. I'll ask a few questions",
    "to personalize your agent. You can always change these",
    "later by editing workspace/USER.md and workspace/TOOLS.md.",
    "",
    "Press Enter to begin.",
];

pub fn render(f: &mut Frame, state: &OnboardingState) {
    let area = f.area();

    // Dark background
    f.render_widget(Clear, area);
    let bg = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(bg, area);

    match state.step {
        OnboardingStep::Welcome => render_welcome(f, state, area),
        OnboardingStep::Confirm => render_confirm(f, state, area),
        OnboardingStep::Writing => render_writing(f, state, area),
        OnboardingStep::Done => render_done(f, area),
        _ => render_input_step(f, state, area),
    }
}

fn center_area(area: Rect, width: u16, height: u16) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vert[1]);
    horiz[1]
}

fn render_welcome(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let content_height = LOGO.len() as u16 + 2 + WELCOME_LINES.len() as u16 + 2;
    let centered = center_area(area, 60, content_height);

    let mut lines: Vec<Line> = Vec::new();

    // Logo with color
    for logo_line in LOGO {
        lines.push(Line::from(Span::styled(
            *logo_line,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
    }

    // Tagline
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {TAGLINE}"),
        Style::default().fg(Color::DarkGray),
    )));

    // Welcome text with typing effect
    let chars_visible = (state.tick * 2) as usize;
    let mut total_chars = 0;
    for welcome_line in WELCOME_LINES {
        if total_chars >= chars_visible {
            break;
        }
        let visible_len = (chars_visible - total_chars).min(welcome_line.len());
        let visible = &welcome_line[..visible_len];
        let color = if welcome_line.contains("Enter") {
            Color::Green
        } else {
            Color::White
        };
        lines.push(Line::from(Span::styled(
            format!("  {visible}"),
            Style::default().fg(color),
        )));
        total_chars += welcome_line.len();
    }

    let paragraph = Paragraph::new(lines).alignment(Alignment::Left);
    f.render_widget(paragraph, centered);
}

fn render_progress(step: &OnboardingStep) -> Line<'static> {
    let current = step.index();
    let total = OnboardingStep::total();
    let mut spans = vec![Span::styled("  ", Style::default())];

    for i in 0..total {
        let dot = if i < current {
            Span::styled(
                " ● ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else if i == current {
            Span::styled(
                " ◉ ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(" ○ ", Style::default().fg(Color::DarkGray))
        };
        spans.push(dot);
    }

    Line::from(spans)
}

fn render_input_step(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let centered = center_area(area, 60, 14);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // progress
            Constraint::Length(1), // spacer
            Constraint::Length(1), // step label
            Constraint::Length(1), // spacer
            Constraint::Length(1), // prompt
            Constraint::Length(1), // spacer
            Constraint::Length(3), // input box
            Constraint::Length(1), // spacer
            Constraint::Length(1), // hint
            Constraint::Length(1), // spacer
            Constraint::Length(1), // nav help
        ])
        .split(centered);

    // Progress dots
    let progress = render_progress(&state.step);
    f.render_widget(Paragraph::new(progress), layout[0]);

    // Step label
    let step_num = state.step.index();
    let step_label = Line::from(vec![
        Span::styled(
            format!("  Step {step_num}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" of {}", OnboardingStep::total() - 1),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(step_label), layout[2]);

    // Prompt
    let prompt = Line::from(Span::styled(
        format!("  {}", state.step.prompt()),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(Paragraph::new(prompt), layout[4]);

    // Input box
    let input = Paragraph::new(state.input.as_str())
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::horizontal(1)),
        );
    f.render_widget(input, layout[6]);

    // Cursor
    let x = layout[6].x + state.cursor_position as u16 + 2;
    let y = layout[6].y + 1;
    f.set_cursor_position((x, y));

    // Hint
    if !state.step.hint().is_empty() {
        let hint = Line::from(Span::styled(
            format!("  {}", state.step.hint()),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ));
        f.render_widget(Paragraph::new(hint), layout[8]);
    }

    // Navigation help
    let nav = Line::from(vec![
        Span::styled("  Enter", Style::default().fg(Color::Green)),
        Span::styled(" next  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Tab", Style::default().fg(Color::Yellow)),
        Span::styled(" skip  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Shift+Tab", Style::default().fg(Color::Yellow)),
        Span::styled(" back  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(nav), layout[10]);
}

fn render_confirm(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let centered = center_area(area, 60, 22);

    let mut lines: Vec<Line> = vec![
        render_progress(&state.step),
        Line::from(""),
        Line::from(Span::styled(
            "  All set. Here's what I've got:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // Summary fields
    let fields = [
        ("Name", &state.data.name),
        ("Pronouns", &state.data.pronouns),
        ("Timezone", &state.data.timezone),
        ("Project", &state.data.projects),
        ("Style", &state.data.comm_style),
        ("Shell", &state.data.shell),
        ("Editor", &state.data.editor),
    ];

    for (label, value) in fields {
        let display = if value.is_empty() {
            "—".to_string()
        } else {
            value.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {label:>10}  "),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(display, Style::default().fg(Color::White)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Files to be written
    lines.push(Line::from(Span::styled(
        "  This will write to:",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "    workspace/USER.md",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "    workspace/TOOLS.md",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(""));

    // Actions
    lines.push(Line::from(vec![
        Span::styled("  Enter", Style::default().fg(Color::Green)),
        Span::styled(" confirm  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Shift+Tab", Style::default().fg(Color::Yellow)),
        Span::styled(" go back  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Red)),
        Span::styled(" quit", Style::default().fg(Color::DarkGray)),
    ]));

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(paragraph, centered);
}

fn render_writing(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let centered = center_area(area, 40, 5);

    let frame = state.tick % 4;
    let spinner = match frame {
        0 => "⠋",
        1 => "⠙",
        2 => "⠹",
        _ => "⠸",
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {spinner} Writing workspace files..."),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, centered);
}

fn render_done(f: &mut Frame, area: Rect) {
    let centered = center_area(area, 50, 7);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  ✓ Setup complete",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Starting gclaw...",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
    ];

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, centered);
}
