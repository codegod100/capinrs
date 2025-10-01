use capnweb_core::{CapId, RpcError, async_trait};
use capnweb_server::{CapTable, RpcTarget, Server, ServerConfig};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

mod websocket_client;
mod websocket_server;

const CALCULATOR_CAP_ID: u64 = 1;
const CHAT_CAP_ID: u64 = 2;
const SESSION_CAP_START: u64 = 10_000;

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
}

impl ChatService {
    fn new(cap_table: Arc<CapTable>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ChatState::with_defaults())),
            cap_table,
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

#[derive(Clone)]
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

    fn validate_credentials(&self, username: &str, password: &str) -> bool {
        self.credentials
            .get(username)
            .map(|stored| stored == password)
            .unwrap_or(false)
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
}

impl ChatSessionCapability {
    fn new(state: Arc<Mutex<ChatState>>, username: String) -> Self {
        Self { state, username }
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

                let mut state = self.state.lock().await;
                state.record_message(&self.username, message);

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::default();
    let server = Server::new(config);

    server.register_capability(CapId::new(CALCULATOR_CAP_ID), Arc::new(Calculator::new()));
    server.register_capability(
        CapId::new(CHAT_CAP_ID),
        Arc::new(ChatService::new(Arc::clone(server.cap_table()))),
    );

    server.run().await?;
    Ok(())
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
