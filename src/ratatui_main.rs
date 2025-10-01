use std::error::Error;
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::Arc;

mod ratatui_client;
mod websocket_client;

use ratatui_client::{RatatuiClient, Session, ChatMessage};
use websocket_client::WebSocketClient;

fn usage() {
    println!(
        "Usage: {} [OPTIONS]

Options:
  --url <URL>    Override the Cap'n Web endpoint
  -h, --help     Show this message

Environment:
  CAPINRS_SERVER_HOST   Override the default backend (wss://capinrs-server.veronika-m-winters.workers.dev)

After launch you'll be prompted for username/password, then you can start chatting!",
        std::env::args().next().unwrap_or("ratatui-client".to_string())
    );
}

fn parse_cli() -> Result<CliOptions, Box<dyn Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    let mut url = std::env::var("CAPINRS_SERVER_HOST")
        .unwrap_or_else(|_| "wss://capinrs-server.veronika-m-winters.workers.dev".to_string());

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--url" => {
                if i + 1 < args.len() {
                    url = args[i + 1].clone();
                    i += 2;
                } else {
                    return Err("--url requires a value".into());
                }
            }
            "-h" | "--help" => {
                usage();
                std::process::exit(0);
            }
            _ => {
                return Err(format!("Unknown argument: {}", args[i]).into());
            }
        }
    }

    Ok(CliOptions { url })
}

struct CliOptions {
    url: String,
}

fn prompt(message: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    use std::io::{self, Write};
    print!("{}", message);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let options = match parse_cli() {
        Ok(opts) => opts,
        Err(err) => {
            eprintln!("Error: {}", err);
            usage();
            std::process::exit(1);
        }
    };

    let url = options.url.clone();
    println!("Connecting to {}", url);

    let username = prompt("Username: ")?;
    let password = prompt("Password: ")?;

    let client = WebSocketClient::new(&url).await.map_err(|e| format!("Failed to connect to WebSocket: {}", e))?;

    let capability = match client.authenticate(&username, &password).await {
        Ok(cap) => cap,
        Err(err) => {
            eprintln!("Authentication failed: {}", err);
            std::process::exit(1);
        }
    };

    let session = Session {
        username,
        capability,
    };

    // Create UI
    let mut ui = RatatuiClient::new()?;
    
    // Set initial status
    ui.set_status(format!("Connected as {} | Type /help for commands | Press Ctrl+C to quit", session.username), false);

    // Load existing messages (calculate how many fit in terminal)
    match client.receive_messages(session.capability).await {
        Ok(messages) => {
            let total_messages = messages.len();
            
            // Calculate how many messages can fit in the terminal
            // Terminal height - 3 (input) - 3 (status) - 2 (borders) = available height
            let terminal_height = ui.get_terminal_size().1 as usize;
            let available_height = terminal_height.saturating_sub(8); // Reserve space for UI elements
            let messages_to_show = available_height.max(5).min(total_messages); // At least 5, at most all messages
            
            let start_index = if total_messages > messages_to_show {
                total_messages - messages_to_show
            } else {
                0
            };
            
            for msg in messages.iter().skip(start_index) {
                ui.add_message(msg.clone().into());
            }
            
            ui.set_status(format!("Connected as {} | Loaded {} recent messages (of {} total) | Type /help for commands | Press Ctrl+C to quit", 
                session.username, ui.message_count(), total_messages), false);
        }
        Err(e) => {
            ui.set_status(format!("Connected as {} | Failed to load messages: {} | Type /help for commands | Press Ctrl+C to quit", session.username, e), true);
        }
    }

    // Spawn task to handle incoming messages
    let message_rx = client.get_message_receiver();
    let ui_messages = Arc::new(tokio::sync::Mutex::new(Vec::<ChatMessage>::new()));
    let ui_messages_clone = ui_messages.clone();
    
    tokio::spawn(async move {
        let mut rx = message_rx.lock().await;
        while let Some(msg) = rx.recv().await {
            let mut messages = ui_messages_clone.lock().await;
            messages.push(msg.into());
        }
    });

    // Main UI loop
    loop {
        // Check for new messages
        {
            let messages = ui_messages.lock().await;
            for msg in messages.iter() {
                // Calculate the current terminal size and message limit
                let terminal_height = ui.get_terminal_size().1 as usize;
                let available_height = terminal_height.saturating_sub(8);
                let max_messages = available_height.max(5);
                
                ui.add_message_with_limit(msg.clone(), max_messages);
            }
        }
        {
            let mut messages = ui_messages.lock().await;
            messages.clear();
        }

        // Draw UI
        ui.draw()?;

        // Handle events
        if ui.handle_event()? {
            if ui.should_quit() {
                break;
            }
            
            // Handle command
            let input = ui.get_input();
            if !input.trim().is_empty() {
                // Add timeout to prevent hanging
                match tokio::time::timeout(
                    tokio::time::Duration::from_secs(5),
                    handle_command(&input, &client, &session, &mut ui)
                ).await {
                    Ok(_) => {
                        // Command completed successfully
                    }
                    Err(_) => {
                        // Command timed out
                        ui.set_status("Command timed out - connection may be lost".to_string(), true);
                    }
                }
            }
        }

        // Small delay to prevent busy waiting
        tokio::time::sleep(tokio::time::Duration::from_millis(16)).await;
    }

    println!("Goodbye!");
    Ok(())
}

async fn handle_command(input: &str, client: &WebSocketClient, session: &Session, ui: &mut RatatuiClient) {
    let trimmed = input.trim();
    
    if !trimmed.starts_with('/') {
        // Send message
        match client.send_message(session.capability, trimmed).await {
            Ok(_) => {
                ui.set_status(format!("Connected as {}", session.username), false);
            }
            Err(e) => {
                ui.set_status(format!("Failed to send message: {}", e), true);
            }
        }
        return;
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let command = parts[0];

    match command {
        "/quit" | "/exit" => {
            ui.quit();
        }
        "/help" => {
            ui.add_message(ChatMessage {
                from: "System".to_string(),
                body: "Commands:
  /help                  Show this help
  /whoami                Show current session
  /receive               Fetch and display messages
  /quit                  Exit the client
Messages without a leading slash are broadcast to the chat.".to_string(),
                timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            });
        }
        "/whoami" => {
            match client.whoami(session.capability).await {
                Ok(result) => {
                    ui.add_message(ChatMessage {
                        from: "System".to_string(),
                        body: format!("You are: {:?}", result),
                        timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
                    });
                }
                Err(e) => {
                    ui.set_status(format!("Whoami failed: {}", e), true);
                }
            }
        }
        "/receive" => {
            match client.receive_messages(session.capability).await {
                Ok(messages) => {
                    for msg in messages {
                        ui.add_message(msg.into());
                    }
                    ui.set_status("Fetched recent messages".to_string(), false);
                }
                Err(e) => {
                    ui.set_status(format!("Failed to receive messages: {}", e), true);
                }
            }
        }
        _ => {
            ui.add_message(ChatMessage {
                from: "System".to_string(),
                body: format!("Unknown command `{}`. Type /help for a list of commands.", command),
                timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            });
        }
    }
}