use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use serde_json::Value;
use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

use crate::websocket_client::WebSocketClient;
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

pub struct ChatUI {
    app: ChatApp,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    message_rx: Arc<Mutex<mpsc::UnboundedReceiver<ChatMessage>>>,
}

impl ChatUI {
    pub fn new(
        message_rx: Arc<Mutex<mpsc::UnboundedReceiver<ChatMessage>>>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            app: ChatApp::new(),
            terminal,
            message_rx,
        })
    }

    pub fn set_status(&mut self, status: String, is_error: bool) {
        self.app.set_status(status, is_error);
    }

    pub async fn run(
        &mut self,
        client: Arc<WebSocketClient>,
        session: Session,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Spawn task to handle incoming messages
        let message_rx = self.message_rx.clone();
        let app_messages = Arc::new(Mutex::new(Vec::<ChatMessage>::new()));
        let app_messages_clone = app_messages.clone();

        tokio::spawn(async move {
            let mut rx = message_rx.lock().unwrap();
            while let Some(msg) = rx.recv().await {
                let mut messages = app_messages_clone.lock().unwrap();
                messages.push(msg);
            }
        });

        // Main UI loop
        loop {
            // Check for new messages
            {
                let messages = app_messages.lock().unwrap();
                for msg in messages.iter() {
                    self.app.add_message(msg.clone());
                }
            }
            {
                let mut messages = app_messages.lock().unwrap();
                messages.clear();
            }

            // Draw UI
            self.terminal.draw(|f| self.ui(f))?;

            // Handle events
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if self.app.handle_input(key) {
                        if self.app.should_quit {
                            break;
                        }

                        // Handle command
                        let input = self.app.get_input();
                        if !input.trim().is_empty() {
                            self.handle_command(&input, &client, &session).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn ui(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // Messages area
                Constraint::Length(3), // Input area
                Constraint::Length(1), // Status bar
            ])
            .split(f.size());

        // Messages area
        let messages: Vec<ListItem> = self
            .app
            .messages
            .iter()
            .map(|msg| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        &msg.from,
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(": "),
                    Span::raw(&msg.body),
                ]))
            })
            .collect();

        let messages_list = List::new(messages)
            .block(Block::default().borders(Borders::ALL).title("Messages"))
            .style(Style::default().fg(Color::Cyan));

        f.render_widget(messages_list, chunks[0]);

        // Input area
        let input_paragraph = Paragraph::new(self.app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title("Input"))
            .style(Style::default().fg(Color::Yellow))
            .wrap(Wrap { trim: true });

        f.render_widget(input_paragraph, chunks[1]);

        // Status bar
        let status_color = if self.app.is_error {
            Color::Red
        } else {
            Color::Blue
        };
        let status_paragraph = Paragraph::new(self.app.status.as_str())
            .block(Block::default().borders(Borders::ALL))
            .style(Style::default().fg(Color::White).bg(status_color));

        f.render_widget(status_paragraph, chunks[2]);
    }

    async fn handle_command(&mut self, input: &str, client: &WebSocketClient, session: &Session) {
        let trimmed = input.trim();

        if !trimmed.starts_with('/') {
            // Send message
            match client.send_message(session.capability, trimmed).await {
                Ok(_) => {
                    self.app
                        .set_status(format!("Connected as {}", session.username), false);
                }
                Err(e) => {
                    self.app
                        .set_status(format!("Failed to send message: {}", e), true);
                }
            }
            return;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let command = parts[0];

        match command {
            "/quit" | "/exit" => {
                self.app.should_quit = true;
            }
            "/help" => {
                self.app.add_message(ChatMessage {
                    from: "System".to_string(),
                    body: "Commands:
  /help                  Show this help
  /whoami                Show current session
  /receive               Fetch and display messages
  /quit                  Exit the client
Messages without a leading slash are broadcast to the chat."
                        .to_string(),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
            }
            "/whoami" => match client.whoami(session.capability).await {
                Ok(result) => {
                    if let Some(username) = result.get("username").and_then(|v| v.as_str()) {
                        self.app.add_message(ChatMessage {
                            from: "System".to_string(),
                            body: format!("You are {}", username),
                            timestamp: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64,
                        });
                        self.app
                            .set_status(format!("Authenticated as {}", username), false);
                    }
                }
                Err(e) => {
                    self.app.set_status(format!("Whoami failed: {}", e), true);
                }
            },
            "/receive" => match client.receive_messages(session.capability).await {
                Ok(messages) => {
                    for msg in messages {
                        self.app.add_message(msg.into());
                    }
                    self.app
                        .set_status("Fetched recent messages".to_string(), false);
                }
                Err(e) => {
                    self.app
                        .set_status(format!("Failed to receive messages: {}", e), true);
                }
            },
            _ => {
                self.app.add_message(ChatMessage {
                    from: "System".to_string(),
                    body: format!(
                        "Unknown command `{}`. Type /help for a list of commands.",
                        command
                    ),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
            }
        }
    }
}

impl Drop for ChatUI {
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

pub struct Session {
    pub username: String,
    pub capability: CapId,
}
