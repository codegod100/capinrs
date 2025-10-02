use rand::Rng;
use std::error::Error;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

mod ratatui_client;
mod websocket_client;

use ratatui_client::{ChatMessage, RatatuiClient, Session};
use websocket_client::WebSocketClient;

fn usage() {
    println!(
        "Usage: {} [OPTIONS]

Options:
  --url <URL>       Override the Cap'n Web endpoint
  --user <NICK>     Use a specific nickname instead of random generation
  --password <PWD>  Nickname password for NickServ (required when using --user)
  -h, --help        Show this message

Environment:
  CAPINRS_SERVER_HOST   Override the default backend (wss://capinrs-server.veronika-m-winters.workers.dev)

After launch you'll connect with your nickname and can start chatting!
Commands: /help, /whoami, /receive, /nickserv, /quit",
        std::env::args().next().unwrap_or("ratatui-client".to_string())
    );
}

fn parse_cli() -> Result<CliOptions, Box<dyn Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    let mut url = std::env::var("CAPINRS_SERVER_HOST")
        .unwrap_or_else(|_| "wss://capinrs-server.veronika-m-winters.workers.dev".to_string());
    let mut user: Option<String> = None;
    let mut password: Option<String> = None;

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
            "--user" => {
                if i + 1 < args.len() {
                    user = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--user requires a value".into());
                }
            }
            "--password" => {
                if i + 1 < args.len() {
                    password = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--password requires a value".into());
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

    Ok(CliOptions {
        url,
        user,
        password,
    })
}

struct CliOptions {
    url: String,
    user: Option<String>,
    password: Option<String>,
}

fn generate_random_nickname() -> String {
    let adjectives = [
        "Happy", "Clever", "Swift", "Bright", "Calm", "Bold", "Wise", "Kind", "Cool", "Sharp",
    ];
    let nouns = [
        "Cat", "Dog", "Bird", "Fish", "Bear", "Wolf", "Fox", "Lion", "Tiger", "Eagle",
    ];
    let mut rng = rand::thread_rng();
    let adj = adjectives[rng.gen_range(0..adjectives.len())];
    let noun = nouns[rng.gen_range(0..nouns.len())];
    let num = rng.gen_range(100..999);
    format!("{}{}{}", adj, noun, num)
}

const STATUS_HELP: &str = "Type /help for commands | Press Ctrl+C to quit";

fn format_status(nickname: &str, server_url: &str, detail: impl AsRef<str>) -> String {
    let detail = detail.as_ref();
    if detail.is_empty() {
        format!("Server: {} | Nick: {}", server_url, nickname)
    } else {
        format!("Server: {} | Nick: {} | {}", server_url, nickname, detail)
    }
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

    if options.user.is_some() && options.password.is_none() {
        eprintln!("Error: --user requires --password to be provided.");
        usage();
        std::process::exit(1);
    }

    let url = options.url.clone();

    // Use provided nickname or generate a random one for authentication
    let username = options
        .user
        .clone()
        .unwrap_or_else(generate_random_nickname);

    let client = WebSocketClient::new(&url)
        .await
        .map_err(|e| format!("Failed to connect to WebSocket: {}", e))?;

    let capability = match client.authenticate(&username, "").await {
        Ok(cap) => cap,
        Err(err) => {
            eprintln!("Authentication failed: {}", err);
            std::process::exit(1);
        }
    };

    let mut session = Session {
        username: username.clone(),
        nickname: username,
        capability,
    };

    // Create UI
    let mut ui = RatatuiClient::new()?;

    // Set initial status
    ui.set_status(
        format_status(&session.nickname, url.as_str(), STATUS_HELP),
        false,
    );

    // Test log RPC call
    ui.log(&client, session.capability, "Client connected successfully")
        .await;

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

            ui.set_status(
                format_status(
                    &session.nickname,
                    url.as_str(),
                    format!(
                        "Loaded {} recent messages (of {} total) | {}",
                        ui.message_count(),
                        total_messages,
                        STATUS_HELP
                    ),
                ),
                false,
            );
        }
        Err(e) => {
            ui.set_status(
                format_status(
                    &session.nickname,
                    url.as_str(),
                    format!("Failed to load messages: {} | {}", e, STATUS_HELP),
                ),
                true,
            );
        }
    }

    // If the user supplied a nickname and password, attempt automatic NickServ identify.
    if let (Some(nick), Some(nick_pwd)) = (&options.user, &options.password) {
        match client.check_nickname(session.capability, nick).await {
            Ok(true) => {}
            Ok(false) => {
                let message = format!("Nickname '{}' is not registered", nick);
                let detail = format!("{} | {}", message, STATUS_HELP);
                ui.set_status(format_status(&session.nickname, url.as_str(), detail), true);
                ui.add_message(ChatMessage {
                    from: "System".to_string(),
                    body: format!("NickServ identify aborted: {}", message),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
                return Err(message.into());
            }
            Err(err) => {
                let message = format!("Failed to verify nickname '{}': {}", nick, err);
                let detail = format!("{} | {}", message, STATUS_HELP);
                ui.set_status(format_status(&session.nickname, url.as_str(), detail), true);
                ui.add_message(ChatMessage {
                    from: "System".to_string(),
                    body: format!("NickServ identify failed: {}", err),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
                return Err(message.into());
            }
        }
        ui.log(
            &client,
            session.capability,
            &format!("Auto-identifying nickname '{}' via CLI credentials", nick),
        )
        .await;
        match client
            .identify_nickname(session.capability, nick, nick_pwd)
            .await
        {
            Ok(message) => {
                let old_nickname = session.nickname.clone();
                session.nickname = nick.to_string();
                ui.set_status(
                    format_status(
                        &session.nickname,
                        url.as_str(),
                        format!("{} | {}", message, STATUS_HELP),
                    ),
                    false,
                );
                ui.add_message(ChatMessage {
                    from: "System".to_string(),
                    body: format!("{}", message),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
                ui.log(
                    &client,
                    session.capability,
                    &format!(
                        "Auto NickServ identify succeeded; nickname changed from '{}' to '{}'",
                        old_nickname, session.nickname
                    ),
                )
                .await;
            }
            Err(err) => {
                ui.set_status(
                    format_status(
                        &session.nickname,
                        url.as_str(),
                        format!("NickServ identify failed: {} | {}", err, STATUS_HELP),
                    ),
                    true,
                );
                ui.add_message(ChatMessage {
                    from: "System".to_string(),
                    body: format!("NickServ identify failed: {}", err),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
                return Err(format!("NickServ identify failed: {}", err).into());
            }
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

            // Handle password input completion
            if ui.is_password_input_active() {
                ui.log(&client, session.capability, "Password input is active")
                    .await;
                // Check if Enter was pressed (password is ready)
                let input = ui.get_input();
                ui.log(
                    &client,
                    session.capability,
                    &format!(
                        "Password input check - input: '{}', empty: {}",
                        input,
                        input.is_empty()
                    ),
                )
                .await;

                // The key insight: during password input, the regular input should be empty
                // and we should check if the password input has content
                if input.is_empty() {
                    let password_input = ui.get_password_input();
                    let password_input_str = password_input.cloned().unwrap_or_default();
                    let is_some = password_input.is_some();

                    ui.log(
                        &client,
                        session.capability,
                        &format!(
                            "Password input check - password_input: '{}', is_some: {}, is_empty: {}",
                            password_input_str,
                            is_some,
                            password_input_str.is_empty()
                        ),
                    )
                    .await;

                    ui.log(
                        &client,
                        session.capability,
                        &format!(
                            "Checking conditions - is_some: {}, password_input_str.is_empty(): {}, password_input_str: '{}'",
                            is_some,
                            password_input_str.is_empty(),
                            password_input_str
                        ),
                    )
                    .await;

                    if is_some && !password_input_str.is_empty() {
                        ui.log(
                            &client,
                            session.capability,
                            "Enter pressed, finishing password input",
                        )
                        .await;
                        // Get the prompt BEFORE finishing password input (which clears it)
                        let default_prompt = String::new();
                        let prompt = ui.get_password_prompt().unwrap_or(&default_prompt).clone();
                        ui.log(
                            &client,
                            session.capability,
                            &format!("Password prompt: '{}'", prompt),
                        )
                        .await;
                        ui.log(
                            &client,
                            session.capability,
                            &format!("Password prompt length: {}", prompt.len()),
                        )
                        .await;
                        // Get the actual command type from the stored command
                        let command = ui
                            .get_current_password_command()
                            .unwrap_or(&String::new())
                            .clone();
                        ui.log(
                            &client,
                            session.capability,
                            &format!("Current password command: '{}'", command),
                        )
                        .await;
                        let password = ui.finish_password_input();
                        ui.log(
                            &client,
                            session.capability,
                            &format!("finish_password_input returned: {:?}", password),
                        )
                        .await;
                        if let Some(pwd) = password {
                            ui.log(
                                &client,
                                session.capability,
                                &format!(
                                    "Password received, length: {}, content: '{}'",
                                    pwd.len(),
                                    pwd
                                ),
                            )
                            .await;
                            if command == "identify" {
                                ui.log(
                                    &client,
                                    session.capability,
                                    "Command is 'identify', proceeding with identification",
                                )
                                .await;
                                // Extract nickname from prompt and call identify
                                ui.log(
                                    &client,
                                    session.capability,
                                    &format!("Looking for nickname in prompt: '{}'", prompt),
                                )
                                .await;
                                if let Some(nick_start) = prompt.find("'") {
                                    ui.log(
                                        &client,
                                        session.capability,
                                        &format!("Found first quote at position: {}", nick_start),
                                    )
                                    .await;
                                    if let Some(nick_end) = prompt.rfind("'") {
                                        ui.log(
                                            &client,
                                            session.capability,
                                            &format!("Found last quote at position: {}", nick_end),
                                        )
                                        .await;
                                        if nick_end > nick_start {
                                            let nick = &prompt[nick_start + 1..nick_end];
                                            ui.log(
                                                &client,
                                                session.capability,
                                                &format!("Extracted nickname: '{}'", nick),
                                            )
                                            .await;
                                            ui.log(&client, session.capability, &format!("Attempting to identify nickname '{}' with password", nick)).await;
                                            ui.log(&client, session.capability, &format!("Calling identify_nickname with nick='{}', password='{}'", nick, pwd)).await;
                                            match client
                                                .identify_nickname(session.capability, nick, &pwd)
                                                .await
                                            {
                                                Ok(message) => {
                                                    ui.log(&client, session.capability, &format!("Identify successful! Server response: {}", message)).await;
                                                    // Update session nickname to the identified nickname
                                                    let old_nickname = session.nickname.clone();
                                                    session.nickname = nick.to_string();
                                                    ui.log(
                                                        &client,
                                                        session.capability,
                                                        &format!(
                                                            "CHANGING NICKNAME: '{}' -> '{}'",
                                                            old_nickname, session.nickname
                                                        ),
                                                    )
                                                    .await;
                                                    ui.set_status(
                                                        format_status(
                                                            &session.nickname,
                                                            url.as_str(),
                                                            STATUS_HELP,
                                                        ),
                                                        false,
                                                    );
                                                    ui.add_message(ChatMessage {
                                                        from: "System".to_string(),
                                                        body: format!(
                                                            "{} - Your display name is now '{}'",
                                                            message, nick
                                                        ),
                                                        timestamp: SystemTime::now()
                                                            .duration_since(UNIX_EPOCH)
                                                            .unwrap()
                                                            .as_millis()
                                                            as u64,
                                                    });
                                                }
                                                Err(e) => {
                                                    ui.log(
                                                        &client,
                                                        session.capability,
                                                        &format!(
                                                            "Identify failed with error: {}",
                                                            e
                                                        ),
                                                    )
                                                    .await;
                                                    ui.add_message(ChatMessage {
                                                        from: "System".to_string(),
                                                        body: format!(
                                                            "Identification failed: {}",
                                                            e
                                                        ),
                                                        timestamp: SystemTime::now()
                                                            .duration_since(UNIX_EPOCH)
                                                            .unwrap()
                                                            .as_millis()
                                                            as u64,
                                                    });
                                                }
                                            }
                                        } else {
                                            ui.log(&client, session.capability, "Nickname extraction failed: nick_end <= nick_start").await;
                                        }
                                    } else {
                                        ui.log(
                                            &client,
                                            session.capability,
                                            "Nickname extraction failed: no closing quote found",
                                        )
                                        .await;
                                    }
                                } else {
                                    ui.log(
                                        &client,
                                        session.capability,
                                        "Nickname extraction failed: no opening quote found",
                                    )
                                    .await;
                                }
                            } else if command == "register" {
                                // Extract nickname from prompt and call register
                                if let Some(nick_start) = prompt.find("'") {
                                    if let Some(nick_end) = prompt.rfind("'") {
                                        if nick_end > nick_start {
                                            let nick = &prompt[nick_start + 1..nick_end];
                                            match client
                                                .register_nickname(session.capability, nick, &pwd)
                                                .await
                                            {
                                                Ok(message) => {
                                                    // Update session nickname to the registered nickname
                                                    let old_nickname = session.nickname.clone();
                                                    session.nickname = nick.to_string();
                                                    ui.log(
                                                        &client,
                                                        session.capability,
                                                        &format!(
                                                            "CHANGING NICKNAME: '{}' -> '{}'",
                                                            old_nickname, session.nickname
                                                        ),
                                                    )
                                                    .await;
                                                    ui.set_status(
                                                        format_status(
                                                            &session.nickname,
                                                            url.as_str(),
                                                            STATUS_HELP,
                                                        ),
                                                        false,
                                                    );
                                                    ui.add_message(ChatMessage {
                                                        from: "System".to_string(),
                                                        body: format!(
                                                            "{} - Your display name is now '{}'",
                                                            message, nick
                                                        ),
                                                        timestamp: SystemTime::now()
                                                            .duration_since(UNIX_EPOCH)
                                                            .unwrap()
                                                            .as_millis()
                                                            as u64,
                                                    });
                                                }
                                                Err(e) => {
                                                    ui.add_message(ChatMessage {
                                                        from: "System".to_string(),
                                                        body: format!("Registration failed: {}", e),
                                                        timestamp: SystemTime::now()
                                                            .duration_since(UNIX_EPOCH)
                                                            .unwrap()
                                                            .as_millis()
                                                            as u64,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            ui.log(&client, session.capability, &format!("Conditions not met - is_some: {}, password_input_str.is_empty(): {}", is_some, password_input_str.is_empty())).await;
                        }
                    }
                }
            } else {
                // Handle regular command
                let input = ui.get_input();
                ui.log(
                    &client,
                    session.capability,
                    &format!("Regular input received: '{}'", input),
                )
                .await;
                if !input.trim().is_empty() {
                    ui.log(&client, session.capability, "Processing non-empty input")
                        .await;
                    // Add timeout to prevent hanging
                    match tokio::time::timeout(
                        tokio::time::Duration::from_secs(5),
                        handle_command(&input, &client, &mut session, &mut ui, url.as_str()),
                    )
                    .await
                    {
                        Ok(_) => {
                            // Command completed successfully
                            ui.add_to_history(input.clone());
                        }
                        Err(_) => {
                            // Command timed out
                            ui.set_status(
                                format_status(
                                    &session.nickname,
                                    url.as_str(),
                                    "Command timed out - connection may be lost",
                                ),
                                true,
                            );
                        }
                    }
                }
            }
        }

        // Small delay to prevent busy waiting
        tokio::time::sleep(tokio::time::Duration::from_millis(16)).await;
    }

    Ok(())
}

async fn handle_command(
    input: &str,
    client: &WebSocketClient,
    session: &mut Session,
    ui: &mut RatatuiClient,
    server_url: &str,
) {
    let trimmed = input.trim();

    // Log every command
    ui.log(
        &client,
        session.capability,
        &format!("Command received: '{}'", trimmed),
    )
    .await;

    if !trimmed.starts_with('/') {
        // Send message
        match client.send_message(session.capability, trimmed).await {
            Ok(_) => {
                ui.set_status(
                    format_status(&session.nickname, server_url, STATUS_HELP),
                    false,
                );
            }
            Err(e) => {
                ui.set_status(
                    format_status(
                        &session.nickname,
                        server_url,
                        format!("Failed to send message: {}", e),
                    ),
                    true,
                );
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
                body: "Available Commands:
  /help                  Show this help
  /whoami                Show current session
  /receive               Fetch and display messages
/nickserv identify <nick>  Identify with a protected nickname
/nickserv register <nick>  Register a new nickname
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
                ui.add_message(ChatMessage {
                    from: "System".to_string(),
                    body: format!("You are: {:?}", result),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
            }
            Err(e) => {
                ui.set_status(
                    format_status(
                        &session.nickname,
                        server_url,
                        format!("Whoami failed: {}", e),
                    ),
                    true,
                );
            }
        },
        "/receive" => match client.receive_messages(session.capability).await {
            Ok(messages) => {
                for msg in messages {
                    ui.add_message(msg.into());
                }
                ui.set_status(
                    format_status(&session.nickname, server_url, "Fetched recent messages"),
                    false,
                );
            }
            Err(e) => {
                ui.set_status(
                    format_status(
                        &session.nickname,
                        server_url,
                        format!("Failed to receive messages: {}", e),
                    ),
                    true,
                );
            }
        },
        "/nickserv" => {
            // Add a system message to show the command was received
            ui.add_message(ChatMessage {
                from: "Debug".to_string(),
                body: format!(
                    "DEBUG: /nickserv command received with {} parts",
                    parts.len()
                ),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            });
            ui.log(
                &client,
                session.capability,
                &format!(
                    "/nickserv command received with {} parts: {:?}",
                    parts.len(),
                    parts
                ),
            )
            .await;
            if parts.len() < 2 {
                ui.add_message(ChatMessage {
                    from: "System".to_string(),
                    body: "NickServ Commands:
/nickserv identify <nick>  Identify with a protected nickname
/nickserv register <nick>  Register a new nickname"
                        .to_string(),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                });
                return;
            }

            let subcommand = parts[1];
            match subcommand {
                "identify" => {
                    ui.add_message(ChatMessage {
                        from: "Debug".to_string(),
                        body: "DEBUG: /nickserv identify subcommand received".to_string(),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                    });
                    ui.log(
                        &client,
                        session.capability,
                        "/nickserv identify subcommand received",
                    )
                    .await;
                    if parts.len() < 3 {
                        ui.add_message(ChatMessage {
                            from: "System".to_string(),
                            body: "Usage: /nickserv identify <nick>
You will be prompted for the nickname password."
                                .to_string(),
                            timestamp: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64,
                        });
                        return;
                    }
                    let nick = parts[2];

                    match client.check_nickname(session.capability, nick).await {
                        Ok(true) => {
                            // Start password input mode
                            ui.log(
                                &client,
                                session.capability,
                                &format!("Starting password input for nickname '{}'", nick),
                            )
                            .await;
                            let prompt_text = format!("Password for nickname '{}'", nick);
                            ui.log(
                                &client,
                                session.capability,
                                &format!("Setting prompt to: '{}'", prompt_text),
                            )
                            .await;
                            ui.start_password_input(prompt_text, "identify".to_string());
                            ui.add_message(ChatMessage {
                                from: "System".to_string(),
                                body: format!(
                                    "Please enter password for nickname '{}' in the input area below",
                                    nick
                                ),
                                timestamp: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap()
                                    .as_millis() as u64,
                            });
                        }
                        Ok(false) => {
                            let message = format!(
                                "Nickname '{}' is not registered. Use /nickserv register <nick>.",
                                nick
                            );
                            let detail = format!("{} | {}", message, STATUS_HELP);
                            ui.set_status(
                                format_status(&session.nickname, server_url, detail),
                                true,
                            );
                            ui.add_message(ChatMessage {
                                from: "System".to_string(),
                                body: message,
                                timestamp: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap()
                                    .as_millis() as u64,
                            });
                            return;
                        }
                        Err(err) => {
                            let message = format!("Failed to verify nickname '{}': {}", nick, err);
                            let detail = format!("{} | {}", message, STATUS_HELP);
                            ui.set_status(
                                format_status(&session.nickname, server_url, detail),
                                true,
                            );
                            ui.add_message(ChatMessage {
                                from: "System".to_string(),
                                body: message,
                                timestamp: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap()
                                    .as_millis() as u64,
                            });
                            return;
                        }
                    }
                }
                "register" => {
                    if parts.len() < 3 {
                        ui.add_message(ChatMessage {
                            from: "System".to_string(),
                            body: "Usage: /nickserv register <nick>
You will be prompted for a password to protect your nickname."
                                .to_string(),
                            timestamp: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64,
                        });
                        return;
                    }
                    let nick = parts[2];

                    // Start password input mode
                    ui.start_password_input(
                        format!("Password for new nickname '{}'", nick),
                        "register".to_string(),
                    );
                    ui.add_message(ChatMessage {
                        from: "System".to_string(),
                        body: format!(
                            "Please enter password for new nickname '{}' in the input area below",
                            nick
                        ),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                    });
                }
                _ => {
                    ui.add_message(ChatMessage {
                        from: "System".to_string(),
                        body: "Unknown nickserv command. Use 'identify' or 'register'".to_string(),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                    });
                }
            }
        }
        _ => {
            ui.add_message(ChatMessage {
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
