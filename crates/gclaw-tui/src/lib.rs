pub mod app;
pub mod event;
pub mod onboarding;
pub mod ui;
pub mod widgets;

use app::App;
use crossterm::event::{EnableMouseCapture, DisableMouseCapture, KeyCode, KeyModifiers, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use event::{AppEvent, EventHandler};
use onboarding::OnboardingStep;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::time::Duration;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    pub fn new() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn restore(&mut self) -> io::Result<()> {
        terminal::disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    pub fn run(
        &mut self,
        app: &mut App,
        events: &mut EventHandler,
        input_tx: &tokio::sync::mpsc::UnboundedSender<app::SubmitResult>,
    ) -> io::Result<()> {
        loop {
            // Only redraw when something changed
            if app.needs_redraw {
                self.terminal.draw(|f| ui::render(f, app))?;
                app.needs_redraw = false;
            }

            // Adaptive poll timeout: fast when animating, slow when idle
            let timeout = if app.is_animating() {
                Duration::from_millis(16)
            } else {
                Duration::from_millis(100)
            };

            if let Some(event) = events.poll_event(timeout) {
                match event {
                    AppEvent::Key(key) => {
                        app.needs_redraw = true;
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
                    AppEvent::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            app.scroll_offset = app.scroll_offset.saturating_sub(3);
                            app.auto_scroll = false;
                            app.needs_redraw = true;
                        }
                        MouseEventKind::ScrollDown => {
                            app.scroll_offset = app.scroll_offset.saturating_add(3);
                            app.needs_redraw = true;
                        }
                        _ => {}
                    },
                    AppEvent::Agent(agent_event) => {
                        app.handle_agent_event(agent_event);
                        // Drain any additional queued agent events before next draw
                        for ev in events.drain_agent_events() {
                            app.handle_agent_event(ev);
                        }
                        app.needs_redraw = true;
                    }
                    AppEvent::Resize(_w, _h) => {
                        app.needs_redraw = true;
                    }
                    AppEvent::Tick => {
                        if let Some(ref mut ob) = app.onboarding {
                            ob.tick();
                            // Only redraw on tick when onboarding has animations
                            if matches!(ob.step, OnboardingStep::Welcome | OnboardingStep::Writing)
                            {
                                app.needs_redraw = true;
                            }
                            // Auto-advance from Writing → Done after a brief pause
                            if ob.step == OnboardingStep::Writing && ob.tick % 30 == 0 {
                                match ob.finalize() {
                                    Ok(()) => {
                                        app.onboarding = None;
                                    }
                                    Err(e) => {
                                        app.onboarding = None;
                                        app.messages.push(app::ChatMessage {
                                            sender: "Error".to_string(),
                                            content: format!(
                                                "Failed to write workspace files: {e}"
                                            ),
                                        });
                                    }
                                }
                                app.needs_redraw = true;
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
        input_tx: &tokio::sync::mpsc::UnboundedSender<app::SubmitResult>,
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
                KeyCode::Up => {
                    app.scroll_offset = app.scroll_offset.saturating_sub(3);
                    app.auto_scroll = false;
                    return;
                }
                KeyCode::Down => {
                    app.scroll_offset = app.scroll_offset.saturating_add(3);
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
                } else {
                    let result = app.submit_input();
                    match &result {
                        app::SubmitResult::None => {}
                        _ => {
                            let _ = input_tx.send(result);
                        }
                    }
                }
            }
            KeyCode::Backspace => app.delete_char(),
            KeyCode::Left => app.move_cursor_left(),
            KeyCode::Right => app.move_cursor_right(),
            KeyCode::PageUp => {
                app.scroll_offset = app.scroll_offset.saturating_sub(10);
                app.auto_scroll = false;
            }
            KeyCode::PageDown => {
                app.scroll_offset = app.scroll_offset.saturating_add(10);
                app.auto_scroll = false;
            }
            KeyCode::Char(c) => app.insert_char(c),
            _ => {}
        }
    }
}
