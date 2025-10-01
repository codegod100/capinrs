use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Terminal,
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;
use capnweb_core::CapId;

#[derive(Clone)]
pub struct ChatMessage {
    pub from: String,
    pub body: String,
    pub timestamp: u64,
}

impl From<crate::websocket_client::ChatMessage> for ChatMessage {
    fn from(msg: crate::websocket_client::ChatMessage) -> Self {
        Self {
            from: msg.from,
            body: msg.body,
            timestamp: msg.timestamp,
        }
    }
}

pub struct ChatApp {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub status: String,
    pub is_error: bool,
    pub should_quit: bool,
}

impl ChatApp {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            status: "Connecting...".to_string(),
            is_error: false,
            should_quit: false,
        }
    }

    pub fn add_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
        // Keep only the last 100 messages to avoid memory issues
        if self.messages.len() > 100 {
            self.messages.remove(0);
        }
    }

    pub fn add_message_with_limit(&mut self, message: ChatMessage, max_messages: usize) {
        self.messages.push(message);
        // Keep only the last max_messages to fit terminal size
        if self.messages.len() > max_messages {
            self.messages.remove(0);
        }
    }

    pub fn set_status(&mut self, status: String, is_error: bool) {
        self.status = status;
        self.is_error = is_error;
    }

    pub fn handle_input(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return true;
            }
            KeyCode::Enter => {
                return true; // Signal that input is ready
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.push(c);
            }
            _ => {}
        }
        false
    }

    pub fn get_input(&mut self) -> String {
        let input = self.input.clone();
        self.input.clear();
        input
    }
}

pub struct RatatuiClient {
    app: ChatApp,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl RatatuiClient {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            app: ChatApp::new(),
            terminal,
        })
    }

    pub fn set_status(&mut self, status: String, is_error: bool) {
        self.app.set_status(status, is_error);
    }

    pub fn add_message(&mut self, message: ChatMessage) {
        self.app.add_message(message);
    }

    pub fn add_message_with_limit(&mut self, message: ChatMessage, max_messages: usize) {
        self.app.add_message_with_limit(message, max_messages);
    }

    pub fn should_quit(&self) -> bool {
        self.app.should_quit
    }

    pub fn draw(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let messages = self.app.messages.clone();
        let input = self.app.input.clone();
        let status = self.app.status.clone();
        let is_error = self.app.is_error;
        
        self.terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1), // Messages area
                    Constraint::Length(3), // Input area
                    Constraint::Length(3), // Status bar
                ])
                .split(f.size());

            // Messages area
            let message_items: Vec<ListItem> = messages
                .iter()
                .map(|msg| {
                    ListItem::new(Line::from(vec![
                        Span::styled(&msg.from, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::raw(": "),
                        Span::raw(&msg.body),
                    ]))
                })
                .collect();

            let messages_list = List::new(message_items)
                .block(Block::default().borders(Borders::ALL).title("Messages"))
                .style(Style::default().fg(Color::Cyan));

            f.render_widget(messages_list, chunks[0]);

            // Input area
            let input_paragraph = Paragraph::new(input.as_str())
                .block(Block::default().borders(Borders::ALL).title("Input"))
                .style(Style::default().fg(Color::Yellow))
                .wrap(Wrap { trim: true });

            f.render_widget(input_paragraph, chunks[1]);

            // Status bar
            let status_color = if is_error { Color::Red } else { Color::Blue };
            let status_paragraph = Paragraph::new(status.as_str())
                .block(Block::default().borders(Borders::ALL).title("Status"))
                .style(Style::default().fg(Color::White).bg(status_color))
                .wrap(Wrap { trim: true });

            f.render_widget(status_paragraph, chunks[2]);
        })?;
        Ok(())
    }

    pub fn handle_event(&mut self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if self.app.handle_input(key) {
                    if self.app.should_quit {
                        return Ok(true);
                    }
                    return Ok(true); // Input ready
                }
            }
        }
        Ok(false)
    }

    pub fn get_input(&mut self) -> String {
        self.app.get_input()
    }

    pub fn quit(&mut self) {
        self.app.should_quit = true;
    }

    pub fn message_count(&self) -> usize {
        self.app.messages.len()
    }

    pub fn get_terminal_size(&self) -> (u16, u16) {
        let size = self.terminal.size().unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
        (size.width, size.height)
    }

}

impl Drop for RatatuiClient {
    fn drop(&mut self) {
        // Restore terminal
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
    }
}

#[derive(Clone)]
pub struct Session {
    pub username: String,
    pub capability: CapId,
}