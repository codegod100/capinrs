use capnweb_core::CapId;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};
use std::io;

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
    pub scroll_state: ScrollbarState,
    pub list_state: ListState,
    pub password_input: Option<String>,
    pub password_prompt: Option<String>,
    pub current_password_command: Option<String>,
    pub command_history: Vec<String>,
    pub history_index: usize,
}

impl ChatApp {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            status: "Connecting...".to_string(),
            is_error: false,
            should_quit: false,
            scroll_state: ScrollbarState::new(0),
            list_state: ListState::default(),
            password_input: None,
            password_prompt: None,
            current_password_command: None,
            command_history: Vec::new(),
            history_index: 0,
        }
    }

    pub fn add_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
        // Keep only the last 100 messages to avoid memory issues
        if self.messages.len() > 100 {
            self.messages.remove(0);
        }
        // Update scroll state to show the latest message
        self.scroll_to_bottom();
    }

    pub fn add_message_with_limit(&mut self, message: ChatMessage, max_messages: usize) {
        self.messages.push(message);
        // Keep only the last max_messages to fit terminal size
        if self.messages.len() > max_messages {
            self.messages.remove(0);
        }
        // Update scroll state to show the latest message
        self.scroll_to_bottom();
    }

    pub fn scroll_up(&mut self) {
        if let Some(selected) = self.list_state.selected() {
            if selected > 0 {
                self.list_state.select(Some(selected - 1));
            }
        }
    }

    pub fn scroll_down(&mut self) {
        let total_items = self.get_total_message_lines();
        if let Some(selected) = self.list_state.selected() {
            if selected < total_items.saturating_sub(1) {
                self.list_state.select(Some(selected + 1));
            }
        } else if total_items > 0 {
            self.list_state.select(Some(0));
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        let total_items = self.get_total_message_lines();
        if total_items > 0 {
            self.list_state.select(Some(total_items.saturating_sub(1)));
        }
    }

    fn get_total_message_lines(&self) -> usize {
        self.messages
            .iter()
            .map(|msg| msg.body.matches('\n').count() + 1)
            .sum()
    }

    pub fn start_password_input(&mut self, prompt: String, command: String) {
        self.password_prompt = Some(prompt);
        self.current_password_command = Some(command);
        self.password_input = Some(String::new());
    }

    pub fn is_password_input_active(&self) -> bool {
        self.password_input.is_some()
    }

    pub fn get_password_prompt(&self) -> Option<&String> {
        self.password_prompt.as_ref()
    }

    pub fn get_password_input(&self) -> Option<&String> {
        self.password_input.as_ref()
    }

    pub fn add_password_char(&mut self, c: char) {
        if let Some(ref mut input) = self.password_input {
            input.push(c);
        }
    }

    pub fn remove_password_char(&mut self) {
        if let Some(ref mut input) = self.password_input {
            input.pop();
        }
    }

    pub fn finish_password_input(&mut self) -> Option<String> {
        let password = self.password_input.clone();
        self.password_input = None;
        self.password_prompt = None;
        self.current_password_command = None;
        password
    }

    pub fn get_current_password_command(&self) -> Option<&String> {
        self.current_password_command.as_ref()
    }

    pub fn add_to_history(&mut self, command: String) {
        // Don't add empty commands or duplicate consecutive commands
        if !command.trim().is_empty() && self.command_history.last() != Some(&command) {
            self.command_history.push(command);
            // Keep only the last 50 commands
            if self.command_history.len() > 50 {
                self.command_history.remove(0);
            }
        }
        self.history_index = self.command_history.len();
    }

    pub fn get_history_previous(&mut self) -> Option<String> {
        if self.history_index > 0 {
            self.history_index -= 1;
            self.command_history.get(self.history_index).cloned()
        } else {
            None
        }
    }

    pub fn get_history_next(&mut self) -> Option<String> {
        if self.history_index < self.command_history.len() {
            self.history_index += 1;
            if self.history_index < self.command_history.len() {
                self.command_history.get(self.history_index).cloned()
            } else {
                Some(String::new()) // Return empty string for "new" command
            }
        } else {
            None
        }
    }

    pub async fn log(
        &mut self,
        client: &crate::websocket_client::WebSocketClient,
        capability: capnweb_core::CapId,
        message: &str,
    ) {
        match client.log(capability, message).await {
            Ok(_) => {
                // Log successful
            }
            Err(e) => {
                // Add error message to UI instead of silently ignoring
                self.add_message(ChatMessage {
                    from: "Log Error".to_string(),
                    body: format!("Log RPC failed: {}", e),
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
            }
        }
    }

    pub fn set_status(&mut self, status: String, is_error: bool) {
        self.status = status;
        self.is_error = is_error;
    }

    pub fn handle_input(&mut self, key: KeyEvent) -> bool {
        // Handle password input mode
        if self.is_password_input_active() {
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                    return true;
                }
                KeyCode::Enter => {
                    return true; // Signal that password is ready
                }
                KeyCode::Backspace => {
                    self.remove_password_char();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.add_password_char(c);
                }
                _ => {}
            }
            return false; // Don't process as regular input
        }

        // Regular input handling
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
            // Command history and scroll handling
            KeyCode::Up => {
                if let Some(history_command) = self.get_history_previous() {
                    self.input = history_command;
                } else {
                    self.scroll_up();
                }
            }
            KeyCode::Down => {
                if let Some(history_command) = self.get_history_next() {
                    self.input = history_command;
                } else {
                    self.scroll_down();
                }
            }
            KeyCode::PageUp => {
                // Scroll up by multiple lines
                for _ in 0..5 {
                    self.scroll_up();
                }
            }
            KeyCode::PageDown => {
                // Scroll down by multiple lines
                for _ in 0..5 {
                    self.scroll_down();
                }
            }
            KeyCode::Home => {
                self.scroll_state = self.scroll_state.position(0);
            }
            KeyCode::End => {
                self.scroll_to_bottom();
            }
            _ => {}
        }
        false
    }

    pub fn get_input(&mut self) -> String {
        if self.is_password_input_active() {
            // Return empty string for password input - it's handled separately
            String::new()
        } else {
            let input = self.input.clone();
            self.input.clear();
            input
        }
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
                    Constraint::Min(1),    // Messages area
                    Constraint::Length(3), // Input area
                    Constraint::Length(3), // Status bar
                ])
                .split(f.size());

            // Messages area with scrollbar
            let message_items: Vec<ListItem> = messages
                .iter()
                .flat_map(|msg| {
                    // Split message body by newlines to handle multi-line messages
                    let lines: Vec<&str> = msg.body.split('\n').collect();
                    lines
                        .into_iter()
                        .enumerate()
                        .map(|(i, line)| {
                            if i == 0 {
                                // First line includes the sender name
                                ListItem::new(Line::from(vec![
                                    Span::styled(
                                        &msg.from,
                                        Style::default()
                                            .fg(Color::Green)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                    Span::raw(": "),
                                    Span::raw(line),
                                ]))
                            } else {
                                // Subsequent lines are indented
                                ListItem::new(Line::from(vec![Span::raw("  "), Span::raw(line)]))
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect();

            // Update scroll state with current content length
            let content_length = message_items.len();
            self.app.scroll_state = self.app.scroll_state.content_length(content_length);

            let messages_list = List::new(message_items)
                .block(Block::default().borders(Borders::ALL).title("Messages"))
                .style(Style::default().fg(Color::Cyan));

            f.render_stateful_widget(messages_list, chunks[0], &mut self.app.list_state);

            // Render scrollbar
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));
            f.render_stateful_widget(scrollbar, chunks[0], &mut self.app.scroll_state);

            // Input area
            let input_text = if let Some(prompt) = self.app.get_password_prompt() {
                let default_input = String::new();
                let password_input = self.app.get_password_input().unwrap_or(&default_input);
                let hidden_password = "*".repeat(password_input.len());
                format!("{}: {}", prompt, hidden_password)
            } else {
                input.clone()
            };

            let input_title = if self.app.is_password_input_active() {
                "Password Input"
            } else {
                "Input"
            };

            let input_paragraph = Paragraph::new(input_text.as_str())
                .block(Block::default().borders(Borders::ALL).title(input_title))
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

    pub fn add_to_history(&mut self, command: String) {
        self.app.add_to_history(command);
    }

    pub fn quit(&mut self) {
        self.app.should_quit = true;
    }

    pub fn message_count(&self) -> usize {
        self.app.messages.len()
    }

    pub fn get_terminal_size(&self) -> (u16, u16) {
        let size = self
            .terminal
            .size()
            .unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
        (size.width, size.height)
    }

    // Password input methods
    pub fn start_password_input(&mut self, prompt: String, command: String) {
        self.app.start_password_input(prompt, command);
    }

    pub fn get_current_password_command(&self) -> Option<&String> {
        self.app.get_current_password_command()
    }

    pub fn is_password_input_active(&self) -> bool {
        self.app.is_password_input_active()
    }

    pub fn get_password_prompt(&self) -> Option<&String> {
        self.app.get_password_prompt()
    }

    pub fn get_password_input(&self) -> Option<&String> {
        self.app.get_password_input()
    }

    pub fn finish_password_input(&mut self) -> Option<String> {
        self.app.finish_password_input()
    }

    pub async fn log(
        &mut self,
        client: &crate::websocket_client::WebSocketClient,
        capability: capnweb_core::CapId,
        message: &str,
    ) {
        self.app.log(client, capability, message).await;
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
    pub nickname: String,
    pub capability: CapId,
}
