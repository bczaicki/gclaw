pub mod app;
pub mod event;
pub mod onboarding;
pub mod ui;
pub mod widgets;

use app::App;
use crossterm::event::{KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use event::{AppEvent, EventHandler};
use onboarding::OnboardingStep;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    pub fn new() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn restore(&mut self) -> io::Result<()> {
        terminal::disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    pub fn run(
        &mut self,
        app: &mut App,
        events: &mut EventHandler,
        input_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    ) -> io::Result<()> {
        loop {
            self.terminal.draw(|f| ui::render(f, app))?;

            if let Some(event) = events.poll_event() {
                match event {
                    AppEvent::Key(key) => {
                        // Global quit
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('c')
                        {
                            app.should_quit = true;
                        } else if app.is_onboarding() {
                            self.handle_onboarding_key(app, key.code, key.modifiers);
                        } else {
                            self.handle_chat_key(app, key.code, key.modifiers, input_tx);
                        }
                    }
                    AppEvent::Agent(agent_event) => {
                        app.handle_agent_event(agent_event);
                    }
                    AppEvent::Tick => {
                        if let Some(ref mut ob) = app.onboarding {
                            ob.tick();
                            // Auto-advance from Writing → Done after a brief pause
                            if ob.step == OnboardingStep::Writing && ob.tick % 30 == 0 {
                                match ob.finalize() {
                                    Ok(()) => {
                                        // Transition to chat
                                        app.onboarding = None;
                                    }
                                    Err(e) => {
                                        // If write fails, drop to chat with error
                                        app.onboarding = None;
                                        app.messages.push(app::ChatMessage {
                                            sender: "Error".to_string(),
                                            content: format!(
                                                "Failed to write workspace files: {e}"
                                            ),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if app.should_quit {
                return Ok(());
            }
        }
    }

    fn handle_onboarding_key(&self, app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        let ob = match app.onboarding.as_mut() {
            Some(ob) => ob,
            None => return,
        };

        match ob.step {
            OnboardingStep::Welcome => match code {
                KeyCode::Enter => ob.advance(),
                KeyCode::Esc => app.should_quit = true,
                _ => {}
            },
            OnboardingStep::Confirm => match code {
                KeyCode::Enter => {
                    ob.step = OnboardingStep::Writing;
                    ob.tick = 0;
                }
                KeyCode::BackTab => ob.go_back(),
                KeyCode::Esc => app.should_quit = true,
                _ => {}
            },
            OnboardingStep::Writing | OnboardingStep::Done => {}
            _ => {
                // Input steps
                match code {
                    KeyCode::Enter => {
                        // Name is required
                        if ob.step == OnboardingStep::Name && ob.input.trim().is_empty() {
                            return;
                        }
                        ob.advance();
                    }
                    KeyCode::Tab => ob.skip_field(),
                    KeyCode::BackTab => ob.go_back(),
                    KeyCode::Backspace => ob.delete_char(),
                    KeyCode::Left => ob.move_cursor_left(),
                    KeyCode::Right => ob.move_cursor_right(),
                    KeyCode::Esc => app.should_quit = true,
                    KeyCode::Char(c) => {
                        // Shift+Tab comes as BackTab on most terminals,
                        // but handle Shift+char normally
                        if modifiers.contains(KeyModifiers::SHIFT) {
                            ob.insert_char(c.to_uppercase().next().unwrap_or(c));
                        } else {
                            ob.insert_char(c);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle_chat_key(
        &self,
        app: &mut App,
        code: KeyCode,
        modifiers: KeyModifiers,
        input_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    ) {
        let is_ctrl = modifiers.contains(KeyModifiers::CONTROL);

        // Ctrl key bindings for conversation management
        if is_ctrl {
            match code {
                KeyCode::Char('n') => {
                    app.new_conversation();
                    return;
                }
                KeyCode::Char('b') => {
                    app.toggle_sidebar();
                    return;
                }
                KeyCode::Up if app.show_sidebar => {
                    app.prev_conversation();
                    return;
                }
                KeyCode::Down if app.show_sidebar => {
                    app.next_conversation();
                    return;
                }
                _ => {}
            }
        }

        match code {
            KeyCode::Esc => {
                app.should_quit = true;
            }
            KeyCode::Enter => {
                if app.show_sidebar && app.input.is_empty() {
                    // Select the currently highlighted conversation
                    app.switch_conversation(app.active_conversation);
                    app.show_sidebar = false;
                } else if let Some(input) = app.submit_input() {
                    let _ = input_tx.send(input);
                }
            }
            KeyCode::Backspace => app.delete_char(),
            KeyCode::Left => app.move_cursor_left(),
            KeyCode::Right => app.move_cursor_right(),
            KeyCode::PageUp => {
                app.scroll_offset = app.scroll_offset.saturating_sub(10);
            }
            KeyCode::PageDown => {
                app.scroll_offset = app.scroll_offset.saturating_add(10);
            }
            KeyCode::Char(c) => app.insert_char(c),
            _ => {}
        }
    }
}
