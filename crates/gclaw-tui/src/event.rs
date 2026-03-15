use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use gclaw_core::AgentEvent;
use std::time::Duration;

pub enum AppEvent {
    Key(KeyEvent),
    Agent(AgentEvent),
    Tick,
}

pub struct EventHandler {
    agent_rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
}

impl EventHandler {
    pub fn new(agent_rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>) -> Self {
        Self { agent_rx }
    }

    pub fn poll_event(&mut self) -> Option<AppEvent> {
        // Check for agent events first (non-blocking)
        if let Ok(agent_event) = self.agent_rx.try_recv() {
            return Some(AppEvent::Agent(agent_event));
        }

        // Check for crossterm events with timeout
        if event::poll(Duration::from_millis(16)).ok()? {
            if let Ok(CrosstermEvent::Key(key)) = event::read() {
                return Some(AppEvent::Key(key));
            }
        }

        Some(AppEvent::Tick)
    }
}
