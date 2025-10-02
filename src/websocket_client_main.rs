use capnweb_core::CapId;
use std::env;
use std::io::{self, BufRead, Write};
use tokio::sync::mpsc;

mod websocket_client;
use websocket_client::{ChatMessage, WebSocketClient, create_websocket_session};

const DEFAULT_BACKEND: &str = "ws://localhost:8787";

struct CliOptions {
    url: String,
    user: Option<String>,
}

struct Session {
    username: String,
    capability: CapId,
}

enum LoopAction {
    Continue,
    Exit,
}

fn usage() {
    eprintln!(
        "Usage: cargo run --bin websocket-client -- [OPTIONS]\n\n\
         Options:\n\
             --url <URL>    Override the Cap'n Web endpoint\n\
             --user <NICK>  Use a specific nickname instead of random generation\n\
             -h, --help     Show this message\n\n\
         Environment:\n\
             CAPINRS_SERVER_HOST   Override the default backend ({}).\n\n\
         After launch you'll be prompted for username/password, the server will
         hand back a dedicated chat capability, and you can chat interactively.
         Commands: /help, /auth, /receive, /whoami, /nickserv, /quit.",
        DEFAULT_BACKEND
    );
}

fn ensure_scheme(raw: &str, fallback: &str) -> String {
    if raw.contains("://") {
        raw.to_string()
    } else {
        format!("{}{}", fallback, raw)
    }
}

fn parse_cli() -> Result<CliOptions, String> {
    let mut args = env::args().skip(1).peekable();
    let mut url_override: Option<String> = None;

    while let Some(arg) = args.peek() {
        match arg.as_str() {
            "--help" | "-h" => {
                usage();
                std::process::exit(0);
            }
            "--url" | "--host" => {
                args.next();
                let value = args
                    .next()
                    .ok_or_else(|| "`--url` requires a value".to_string())?;
                url_override = Some(value);
            }
            _ if arg.starts_with('-') => {
                return Err(format!("Unrecognized flag `{}`", arg));
            }
            _ => break,
        }
    }

    if let Some(arg) = args.next() {
        return Err(format!("Unexpected argument `{}`", arg));
    }

    let env_override = env::var("CAPINRS_SERVER_HOST").ok();
    let raw_target = url_override
        .or(env_override)
        .unwrap_or_else(|| DEFAULT_BACKEND.to_string());
    let url = ensure_scheme(&raw_target, "ws://");

    Ok(CliOptions { url })
}

fn prompt(label: &str) -> io::Result<String> {
    print!("{}: ", label);
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn spawn_input_reader() -> mpsc::UnboundedReceiver<String> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        loop {
            let mut line = String::new();
            match handle.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
                    if tx.send(trimmed).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

async fn handle_user_input(
    line: &str,
    client: &WebSocketClient,
    session: &mut Session,
) -> Result<LoopAction, Box<dyn std::error::Error>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(LoopAction::Continue);
    }

    if !trimmed.starts_with('/') {
        client
            .send_message(session.capability, trimmed)
            .await
            .map_err(|e| format!("Send message error: {}", e))?;
        return Ok(LoopAction::Continue);
    }

    let mut parts = trimmed.split_whitespace();
    match parts.next().unwrap_or("") {
        "/quit" | "/exit" => Ok(LoopAction::Exit),
        "/help" => {
            println!(
                "Commands:\n  /help                  Show this help\n  /auth <user> <pass>    Authenticate again\n  /receive               Fetch pending messages\n  /whoami                Show current session\n  /quit                  Exit the client\nMessages without a leading slash are broadcast to the chat."
            );
            Ok(LoopAction::Continue)
        }
        "/auth" => {
            let username = parts
                .next()
                .ok_or_else(|| "Usage: /auth <username> <password>".to_string())?;
            let password = parts
                .next()
                .ok_or_else(|| "Usage: /auth <username> <password>".to_string())?;
            let capability = client
                .authenticate(username, password)
                .await
                .map_err(|e| format!("Authentication error: {}", e))?;
            session.username = username.to_string();
            session.capability = capability;
            println!("Re-authenticated as {}", session.username);
            Ok(LoopAction::Continue)
        }
        "/receive" | "/poll" => {
            let messages = client
                .receive_messages(session.capability)
                .await
                .map_err(|e| format!("Receive messages error: {}", e))?;
            for msg in messages {
                println!("{}: {}", msg.from, msg.body);
            }
            Ok(LoopAction::Continue)
        }
        "/whoami" => {
            let username = client
                .whoami(session.capability)
                .await
                .map_err(|e| format!("Whoami error: {}", e))?;
            println!(
                "You are {} with capability {}",
                username,
                session.capability.as_u64()
            );
            Ok(LoopAction::Continue)
        }
        other => {
            println!(
                "Unknown command `{}`. Type /help for a list of commands.",
                other
            );
            Ok(LoopAction::Continue)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = match parse_cli() {
        Ok(opts) => opts,
        Err(err) => {
            eprintln!("Error: {}", err);
            usage();
            std::process::exit(1);
        }
    };

    println!("Connecting to {}", options.url);

    let username = prompt("Username")?;
    let password = prompt("Password")?;

    let client = match create_websocket_session(&options.url).await {
        Ok(client) => client,
        Err(err) => {
            eprintln!("Failed to connect to WebSocket: {}", err);
            std::process::exit(1);
        }
    };

    let capability = match client.authenticate(&username, &password).await {
        Ok(cap) => cap,
        Err(err) => {
            eprintln!("Authentication failed: {}", err);
            std::process::exit(1);
        }
    };

    println!("Welcome, {}! Type /help for available commands.", username);
    let mut session = Session {
        username,
        capability,
    };

    // Set up message handler for real-time messages
    let client_clone = client.get_client();
    client_clone
        .set_on_message(|message: ChatMessage| {
            println!("{}: {}", message.from, message.body);
        })
        .await;

    // Load initial messages
    if let Err(err) = client.receive_messages(session.capability).await {
        eprintln!("Warning: couldn't fetch initial messages: {}", err);
    }

    let mut input_rx = spawn_input_reader();

    while let Some(line) = input_rx.recv().await {
        match handle_user_input(&line, &client, &mut session).await {
            Ok(LoopAction::Continue) => {}
            Ok(LoopAction::Exit) => break,
            Err(err) => eprintln!("Error: {}", err),
        }
    }

    println!("Goodbye!");
    Ok(())
}
