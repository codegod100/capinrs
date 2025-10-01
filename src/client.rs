use capnweb_client::{Client as CapnClient, ClientConfig};
use capnweb_core::CapId;
use serde_json::{Value, json};
use std::convert::TryFrom;
use std::env;
use std::io::{self, BufRead, Write};
use tokio::sync::mpsc;

const DEFAULT_CAPN_BACKEND: &str = "http://localhost:8080";
const RPC_PATH: &str = "/rpc/batch";
const CHAT_CAP_ID: u64 = 2;

struct CliOptions {
    url: String,
}

struct Session {
    username: String,
    capability: CapId,
}

struct ChatLogEntry {
    from: String,
    body: String,
    timestamp: u64,
}

enum LoopAction {
    Continue,
    Exit,
}

fn usage() {
    eprintln!(
        "Usage: cargo run --bin client -- [OPTIONS]\n\n\
         Options:\n\
             --url <URL>    Override the Cap'n Web endpoint\n\
             -h, --help     Show this message\n\
\n\
         Environment:\n\
             CAPINRS_SERVER_HOST   Override the default backend ({}).\n\
\n\
         After launch you'll be prompted for username/password, the server will
         hand back a dedicated chat capability, and you can chat interactively.
         Commands: /help, /auth, /receive, /whoami, /quit.",
        DEFAULT_CAPN_BACKEND
    );
}

fn ensure_scheme(raw: &str, fallback: &str) -> String {
    if raw.contains("://") {
        raw.to_string()
    } else {
        format!("{}{}", fallback, raw)
    }
}

fn normalize_endpoint(raw: &str, default_scheme: &str) -> String {
    let with_scheme = ensure_scheme(raw, default_scheme);
    if with_scheme.ends_with(RPC_PATH) {
        with_scheme
    } else {
        format!(
            "{}/{}",
            with_scheme.trim_end_matches('/'),
            RPC_PATH.trim_start_matches('/')
        )
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
        .unwrap_or_else(|| DEFAULT_CAPN_BACKEND.to_string());
    let url = normalize_endpoint(&raw_target, "http://");

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

async fn authenticate(
    client: &CapnClient,
    username: &str,
    password: &str,
) -> Result<CapId, Box<dyn std::error::Error>> {
    let response = client
        .call(
            CapId::new(CHAT_CAP_ID),
            "auth",
            vec![json!(username), json!(password)],
        )
        .await?;

    let session = response
        .get("session")
        .ok_or("Authentication response missing session capability")?;

    let id_value = session
        .get("id")
        .and_then(Value::as_i64)
        .ok_or("Session capability missing id")?;

    let id = u64::try_from(id_value).map_err(|_| "Session capability id must be non-negative")?;

    Ok(CapId::new(id))
}

async fn send_message(
    client: &CapnClient,
    capability: CapId,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    client
        .call(capability, "sendMessage", vec![json!(message)])
        .await?;
    Ok(())
}

async fn receive_and_display(
    client: &CapnClient,
    capability: CapId,
    last_seen: &mut usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = (client, capability, last_seen);
    Ok(())
}

async fn handle_user_input(
    line: &str,
    client: &CapnClient,
    session: &mut Session,
    last_seen: &mut usize,
) -> Result<LoopAction, Box<dyn std::error::Error>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(LoopAction::Continue);
    }

    if !trimmed.starts_with('/') {
        send_message(client, session.capability, trimmed).await?;
        receive_and_display(client, session.capability, last_seen).await?;
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
            let capability = authenticate(client, username, password).await?;
            session.username = username.to_string();
            session.capability = capability;
            *last_seen = 0;
            println!("Re-authenticated as {}", session.username);
            receive_and_display(client, session.capability, last_seen).await?;
            Ok(LoopAction::Continue)
        }
        "/receive" | "/poll" => {
            receive_and_display(client, session.capability, last_seen).await?;
            Ok(LoopAction::Continue)
        }
        "/whoami" => {
            println!(
                "You are {} with capability {}",
                session.username,
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

    let config = ClientConfig {
        url: options.url.clone(),
        ..Default::default()
    };
    let client = CapnClient::new(config)?;

    let capability = match authenticate(&client, &username, &password).await {
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
    let mut last_seen = 0usize;

    if let Err(err) = receive_and_display(&client, session.capability, &mut last_seen).await {
        eprintln!("Warning: couldn't fetch initial messages: {}", err);
    }

    let mut input_rx = spawn_input_reader();

    while let Some(line) = input_rx.recv().await {
        match handle_user_input(&line, &client, &mut session, &mut last_seen).await {
            Ok(LoopAction::Continue) => {}
            Ok(LoopAction::Exit) => break,
            Err(err) => eprintln!("Error: {}", err),
        }
    }

    println!("Goodbye!");
    Ok(())
}
