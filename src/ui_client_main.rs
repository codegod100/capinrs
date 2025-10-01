use std::sync::Arc;
use std::error::Error;
use tokio::sync::mpsc;

mod ui_client;
mod websocket_client;

use ui_client::{ChatUI, Session};
use websocket_client::WebSocketClient;
use capnweb_core::CapId;

fn usage() {
    println!(
        "Usage: {} [OPTIONS]

Options:
  --url <URL>    Override the Cap'n Web endpoint
  -h, --help     Show this message

Environment:
  CAPINRS_SERVER_HOST   Override the default backend (wss://capinrs-server.veronika-m-winters.workers.dev)

After launch you'll be prompted for username/password, then you can start chatting!",
        std::env::args().next().unwrap_or("ui-client".to_string())
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

    println!("Welcome, {}! Starting UI...", username);
    
    let session = Session {
        username,
        capability,
    };

    // Create message channel for UI
    let (message_tx, message_rx) = mpsc::unbounded_channel();
    let message_rx = Arc::new(std::sync::Mutex::new(message_rx));

    // Create UI
    let mut ui = ChatUI::new(message_rx)?;
    
    // Set initial status
    ui.set_status(format!("Connected as {}", session.username), false);

    // Run the UI
    ui.run(Arc::new(client), session).await?;

    println!("Goodbye!");
    Ok(())
}
