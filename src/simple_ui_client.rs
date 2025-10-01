use std::sync::Arc;
use std::error::Error;
use tokio::sync::mpsc;
use std::io::{self, Write, BufRead, BufReader};

use crate::websocket_client::WebSocketClient;
use capnweb_core::CapId;

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

pub struct Session {
    pub username: String,
    pub capability: CapId,
}

pub struct SimpleUI {
    client: Arc<WebSocketClient>,
    session: Session,
    message_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ChatMessage>>>,
}

impl SimpleUI {
    pub fn new(
        client: Arc<WebSocketClient>,
        session: Session,
        message_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ChatMessage>>>,
    ) -> Self {
        Self {
            client,
            session,
            message_rx,
        }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
        println!("Welcome to the chat! Type /help for commands.");
        println!("Connected as: {}", self.session.username);
        println!("Type messages and press Enter to send them.");
        println!("Type /quit to exit.\n");

        // Spawn task to handle incoming messages
        let message_rx = self.message_rx.clone();

        tokio::spawn(async move {
            let mut rx = message_rx.lock().await;
            while let Some(msg) = rx.recv().await {
                println!("{}: {}", msg.from, msg.body);
            }
        });

        // Main input loop
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin.lock());

        loop {
            print!("> ");
            io::stdout().flush()?;

            let mut input = String::new();
            reader.read_line(&mut input)?;
            let input = input.trim();

            if input.is_empty() {
                continue;
            }

            if input == "/quit" || input == "/exit" {
                println!("Goodbye!");
                break;
            }

            self.handle_input(input).await;
        }

        Ok(())
    }

    async fn handle_input(&mut self, input: &str) {
        if !input.starts_with('/') {
            // Send message
            match self.client.send_message(self.session.capability, input).await {
                Ok(_) => {
                    println!("✓ Message sent");
                }
                Err(e) => {
                    println!("✗ Failed to send message: {}", e);
                }
            }
            return;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();
        let command = parts[0];

        match command {
            "/help" => {
                println!("Commands:");
                println!("  /help                  Show this help");
                println!("  /whoami                Show current session");
                println!("  /receive               Fetch and display messages");
                println!("  /quit                  Exit the client");
                println!("Messages without a leading slash are broadcast to the chat.");
            }
            "/whoami" => {
                match self.client.whoami(self.session.capability).await {
                    Ok(result) => {
                        // Simple approach - just print the result
                        println!("Whoami result: {:?}", result);
                    }
                    Err(e) => {
                        println!("Whoami failed: {}", e);
                    }
                }
            }
            "/receive" => {
                match self.client.receive_messages(self.session.capability).await {
                    Ok(messages) => {
                        println!("Recent messages:");
                        for msg in messages {
                            println!("  {}: {}", msg.from, msg.body);
                        }
                    }
                    Err(e) => {
                        println!("Failed to receive messages: {}", e);
                    }
                }
            }
            _ => {
                println!("Unknown command `{}`. Type /help for a list of commands.", command);
            }
        }
    }
}

impl Clone for Session {
    fn clone(&self) -> Self {
        Self {
            username: self.username.clone(),
            capability: self.capability,
        }
    }
}