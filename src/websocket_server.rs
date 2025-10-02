use capnweb_core::{CapId, RpcError, async_trait};
use capnweb_server::{CapTable, RpcTarget};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const CALCULATOR_CAP_ID: u64 = 1;
const CHAT_CAP_ID: u64 = 2;
const SESSION_CAP_START: u64 = 10_000;

// Client connection info
#[derive(Clone)]
struct ClientConnection {
    id: Uuid,
    sender: mpsc::UnboundedSender<Message>,
}

// Server state with client management
#[derive(Clone)]
pub struct WebSocketServer {
    calculator: Arc<Calculator>,
    chat_service: Arc<ChatService>,
    clients: Arc<Mutex<HashMap<Uuid, ClientConnection>>>,
    message_broadcaster: mpsc::UnboundedSender<ChatMessage>,
}

impl WebSocketServer {
    pub fn new() -> Self {
        let (message_tx, mut message_rx) = mpsc::unbounded_channel();
        let clients = Arc::new(Mutex::new(HashMap::<Uuid, ClientConnection>::new()));
        let clients_clone = clients.clone();

        // Spawn message broadcaster task
        tokio::spawn(async move {
            while let Some(message) = message_rx.recv().await {
                let clients = clients_clone.lock().await;
                for client in clients.values() {
                    let _ = client.sender.send(Message::Text(
                        json!(["push", ["pipeline", 0, ["receiveMessage"], [message]]]).to_string(),
                    ));
                }
            }
        });

        let cap_table = Arc::new(CapTable::new());
        let chat_service = Arc::new(ChatService::new(cap_table.clone(), message_tx.clone()));

        Self {
            calculator: Arc::new(Calculator::new()),
            chat_service,
            clients,
            message_broadcaster: message_tx,
        }
    }

    pub async fn handle_websocket(&self, stream: WebSocketStream<tokio::net::TcpStream>) {
        let (mut ws_sender, mut ws_receiver) = stream.split();
        let client_id = Uuid::new_v4();

        // Handle incoming messages
        let chat_service = self.chat_service.clone();
        let calculator = self.calculator.clone();
        let clients = self.clients.clone();

        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(json_msg) = serde_json::from_str::<Value>(&text) {
                        if let Some(array) = json_msg.as_array() {
                            if array.len() >= 2 {
                                match array[0].as_str() {
                                    Some("push") => {
                                        if let Some(pipeline) = array[1].as_array() {
                                            if pipeline.len() >= 4
                                                && pipeline[0].as_str() == Some("pipeline")
                                            {
                                                let import_id = pipeline[1].as_u64().unwrap_or(0);
                                                let method = pipeline[2]
                                                    .as_array()
                                                    .and_then(|m| m.get(0))
                                                    .and_then(Value::as_str);
                                                let args = if let Some(args_array) =
                                                    pipeline[3].as_array()
                                                {
                                                    args_array.clone()
                                                } else {
                                                    Vec::new()
                                                };

                                                let result = match method {
                                                    Some("auth") => {
                                                        println!("WebSocket server: auth called");
                                                        chat_service.call("auth", args).await
                                                    }
                                                    Some("sendMessage") => {
                                                        println!(
                                                            "WebSocket server: sendMessage called"
                                                        );
                                                        chat_service.call("sendMessage", args).await
                                                    }
                                                    Some("receiveMessages") => {
                                                        println!(
                                                            "WebSocket server: receiveMessages called"
                                                        );
                                                        chat_service
                                                            .call("receiveMessages", args)
                                                            .await
                                                    }
                                                    Some("whoami") => {
                                                        println!("WebSocket server: whoami called");
                                                        chat_service.call("whoami", args).await
                                                    }
                                                    Some("add") => {
                                                        calculator.call("add", args).await
                                                    }
                                                    Some("stats") => {
                                                        calculator.call("stats", args).await
                                                    }
                                                    _ => {
                                                        println!(
                                                            "WebSocket server: unknown method: {:?}",
                                                            method
                                                        );
                                                        Err(RpcError::not_found(
                                                            "Unknown method".to_string(),
                                                        ))
                                                    }
                                                };

                                                // Send response
                                                let response = match result {
                                                    Ok(value) => {
                                                        json!(["resolve", import_id, value])
                                                    }
                                                    Err(error) => {
                                                        json!(["reject", import_id, {"message": error.to_string()}])
                                                    }
                                                };

                                                let _ = ws_sender
                                                    .send(Message::Text(response.to_string()))
                                                    .await;
                                            }
                                        }
                                    }
                                    Some("pull") => {
                                        // Server is requesting a value - respond with null
                                        if array.len() >= 2 {
                                            let pull_id = array[1].as_u64().unwrap_or(0);
                                            let resolve_msg = json!(["resolve", pull_id, null]);
                                            let _ = ws_sender
                                                .send(Message::Text(resolve_msg.to_string()))
                                                .await;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    eprintln!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        // Remove client on disconnect
        {
            let mut clients = clients.lock().await;
            clients.remove(&client_id);
        }
    }
}

// Reuse existing structs from main.rs
struct Calculator {
    state: Arc<Mutex<CalculatorState>>,
}

impl Calculator {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(CalculatorState::default())),
        }
    }
}

#[derive(Default)]
struct CalculatorState {
    call_count: u64,
    last_request: Option<String>,
    last_response: Option<String>,
}

impl CalculatorState {
    fn record_call(&mut self, method: &str, args: &[Value], response: &Value) {
        self.call_count += 1;

        let push_line = json!(["push", ["call", CALCULATOR_CAP_ID, [method], args]]);
        let pull_line = json!(["pull", CALCULATOR_CAP_ID]);
        self.last_request = Some(format!("{}\n{}", push_line, pull_line));

        let result_line = json!(["result", CALCULATOR_CAP_ID, response]);
        self.last_response = Some(result_line.to_string());
    }

    fn snapshot(&self) -> Value {
        json!({
            "callCount": self.call_count,
            "lastRequest": self.last_request,
            "lastResponse": self.last_response,
        })
    }
}

struct ChatService {
    state: Arc<Mutex<ChatState>>,
    cap_table: Arc<CapTable>,
    message_broadcaster: mpsc::UnboundedSender<ChatMessage>,
}

impl ChatService {
    fn new(
        cap_table: Arc<CapTable>,
        message_broadcaster: mpsc::UnboundedSender<ChatMessage>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(ChatState::with_defaults())),
            cap_table,
            message_broadcaster,
        }
    }
}

#[derive(Default)]
struct ChatState {
    credentials: HashMap<String, String>,
    messages: Vec<ChatMessage>,
    next_session_cap_id: u64,
    active_sessions: HashMap<u64, String>,
}

#[derive(Clone, serde::Serialize)]
struct ChatMessage {
    from: String,
    body: String,
    timestamp: u64,
}

impl ChatState {
    fn with_defaults() -> Self {
        let mut state = ChatState {
            credentials: HashMap::new(),
            messages: Vec::new(),
            next_session_cap_id: SESSION_CAP_START,
            active_sessions: HashMap::new(),
        };
        state
            .credentials
            .insert("alice".to_string(), "password123".to_string());
        state
            .credentials
            .insert("bob".to_string(), "hunter2".to_string());
        state
            .credentials
            .insert("carol".to_string(), "letmein".to_string());
        state
    }

    fn validate_credentials(&self, _username: &str, _password: &str) -> bool {
        true
    }

    fn allocate_session_capability(&mut self, username: &str) -> u64 {
        let cap_id = self.next_session_cap_id;
        self.next_session_cap_id = self.next_session_cap_id.saturating_add(1);
        self.active_sessions.insert(cap_id, username.to_string());
        cap_id
    }

    fn record_message(&mut self, from: &str, body: &str) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.messages.push(ChatMessage {
            from: from.to_string(),
            body: body.to_string(),
            timestamp,
        });
    }

    fn messages_snapshot(&self) -> Value {
        let messages: Vec<Value> = self
            .messages
            .iter()
            .map(|msg| {
                json!({
                    "from": msg.from,
                    "body": msg.body,
                    "timestamp": msg.timestamp,
                })
            })
            .collect();

        json!({ "messages": messages })
    }
}

struct ChatSessionCapability {
    state: Arc<Mutex<ChatState>>,
    username: String,
    message_broadcaster: mpsc::UnboundedSender<ChatMessage>,
}

impl ChatSessionCapability {
    fn new(
        state: Arc<Mutex<ChatState>>,
        username: String,
        message_broadcaster: mpsc::UnboundedSender<ChatMessage>,
    ) -> Self {
        Self {
            state,
            username,
            message_broadcaster,
        }
    }
}

#[async_trait]
impl RpcTarget for Calculator {
    async fn call(&self, member: &str, args: Vec<Value>) -> Result<Value, RpcError> {
        match member {
            "add" => {
                let (a, b) = expect_two_numbers(member, &args)?;
                let result_value = json!(a + b);

                {
                    let mut state = self.state.lock().await;
                    state.record_call(member, &args, &result_value);
                }

                Ok(result_value)
            }
            "stats" => {
                let state = self.state.lock().await;
                Ok(state.snapshot())
            }
            _ => Err(RpcError::not_found(format!(
                "method `{}` not found",
                member
            ))),
        }
    }
}

#[async_trait]
impl RpcTarget for ChatSessionCapability {
    async fn call(&self, member: &str, args: Vec<Value>) -> Result<Value, RpcError> {
        match member {
            "sendMessage" => {
                if args.len() != 1 {
                    return Err(RpcError::bad_request(
                        "`sendMessage` expects <message>".to_string(),
                    ));
                }
                let message = args[0]
                    .as_str()
                    .ok_or_else(|| RpcError::bad_request("message must be a string"))?;

                let new_message = ChatMessage {
                    from: self.username.clone(),
                    body: message.to_string(),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };

                // Store message
                {
                    let mut state = self.state.lock().await;
                    state.record_message(&self.username, message);
                }

                // Broadcast to all clients
                let _ = self.message_broadcaster.send(new_message);

                Ok(json!({
                    "status": "ok",
                    "echo": message,
                }))
            }
            "receiveMessages" => {
                if !args.is_empty() {
                    return Err(RpcError::bad_request(
                        "`receiveMessages` does not take arguments".to_string(),
                    ));
                }

                let state = self.state.lock().await;
                Ok(state.messages_snapshot())
            }
            "whoami" => Ok(json!({
                "username": self.username,
            })),
            _ => Err(RpcError::not_found(format!(
                "method `{}` not found",
                member
            ))),
        }
    }
}

#[async_trait]
impl RpcTarget for ChatService {
    async fn call(&self, member: &str, args: Vec<Value>) -> Result<Value, RpcError> {
        match member {
            "auth" => {
                if args.len() != 2 {
                    return Err(RpcError::bad_request(
                        "`auth` expects <username>, <password>".to_string(),
                    ));
                }
                let username = args[0]
                    .as_str()
                    .ok_or_else(|| RpcError::bad_request("username must be a string"))?;
                let password = args[1]
                    .as_str()
                    .ok_or_else(|| RpcError::bad_request("password must be a string"))?;

                let (cap_id, username_owned) = {
                    let mut state = self.state.lock().await;
                    if !state.validate_credentials(username, password) {
                        return Err(RpcError::bad_request("invalid credentials"));
                    }
                    let cap_id = state.allocate_session_capability(username);
                    (cap_id, username.to_string())
                };

                let session_capability: Arc<dyn RpcTarget> = Arc::new(ChatSessionCapability::new(
                    self.state.clone(),
                    username_owned.clone(),
                    self.message_broadcaster.clone(),
                ));

                self.cap_table
                    .insert(CapId::new(cap_id), session_capability);

                let id_as_i64 = i64::try_from(cap_id)
                    .map_err(|_| RpcError::internal("session capability id overflow"))?;

                Ok(json!({
                    "session": {
                        "_type": "capability",
                        "id": id_as_i64,
                    },
                    "user": username_owned,
                }))
            }
            "sendMessage" | "receiveMessages" => Err(RpcError::bad_request(
                "call these methods on the session capability returned by `auth`",
            )),
            _ => Err(RpcError::not_found(format!(
                "method `{}` not found",
                member
            ))),
        }
    }
}

fn expect_two_numbers(method: &str, args: &[Value]) -> Result<(f64, f64), RpcError> {
    if args.len() != 2 {
        return Err(RpcError::bad_request(format!(
            "`{}` expects exactly two numeric arguments",
            method
        )));
    }

    let a = args[0]
        .as_f64()
        .ok_or_else(|| RpcError::bad_request("first argument must be a number"))?;
    let b = args[1]
        .as_f64()
        .ok_or_else(|| RpcError::bad_request("second argument must be a number"))?;

    Ok((a, b))
}
