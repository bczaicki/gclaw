use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, MouseEvent};
use gclaw_core::AgentEvent;
use std::time::Duration;

pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Agent(AgentEvent),
    Resize(u16, u16),
    Tick,
}

pub struct EventHandler {
    agent_rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
}

impl EventHandler {
    pub fn new(agent_rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>) -> Self {
        Self { agent_rx }
    }

    pub fn poll_event(&mut self, timeout: Duration) -> Option<AppEvent> {
        // Check for agent events first (non-blocking)
        if let Ok(agent_event) = self.agent_rx.try_recv() {
            return Some(AppEvent::Agent(agent_event));
        }

        // Check for crossterm events with configurable timeout
        if event::poll(timeout).ok()? {
            match event::read() {
                Ok(CrosstermEvent::Key(key)) => return Some(AppEvent::Key(key)),
                Ok(CrosstermEvent::Mouse(mouse)) => return Some(AppEvent::Mouse(mouse)),
                Ok(CrosstermEvent::Resize(w, h)) => return Some(AppEvent::Resize(w, h)),
                _ => {}
            }
        }

        Some(AppEvent::Tick)
    }

    /// Drain all pending agent events from the channel.
    pub fn drain_agent_events(&mut self) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.agent_rx.try_recv() {
            events.push(ev);
        }
        events
    }
}
