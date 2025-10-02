use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use wasm_bindgen::prelude::*;
use worker::*;

const CALCULATOR_CAP_ID: u64 = 1;
const CHAT_CAP_ID: u64 = 2;
const SESSION_CAP_START: u64 = 10_000;

#[derive(Debug)]
enum PendingOutcome {
    Result(Value),
    Error(String),
}

#[derive(Debug, Clone)]
struct ChatMessage {
    from: String,
    body: String,
    timestamp: u64,
}

#[derive(Debug)]
struct ChatState {
    credentials: HashMap<String, String>,
    messages: Vec<ChatMessage>,
    next_session_cap_id: u64,
    active_sessions: HashMap<u64, String>,
}

impl ChatState {
    fn new() -> Self {
        let mut state = ChatState {
            credentials: HashMap::new(),
            messages: Vec::new(),
            next_session_cap_id: SESSION_CAP_START,
            active_sessions: HashMap::new(),
        };
        
        
        state
    }

    fn validate_credentials(&self, username: &str, password: &str) -> bool {
        // Accept any username with default password
        password == "default_password"
    }

    fn allocate_session_capability(&mut self, username: &str) -> u64 {
        let cap_id = self.next_session_cap_id;
        self.next_session_cap_id = self.next_session_cap_id.saturating_add(1);
        self.active_sessions.insert(cap_id, username.to_string());
        cap_id
    }

    fn record_message(&mut self, from: &str, body: &str) {
        let timestamp = js_sys::Date::now() as u64;
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

#[wasm_bindgen]
pub fn process_rpc(input: &str) -> Result<String, JsValue> {
    process_batch(input).map_err(|err| JsValue::from_str(&err))
}

fn process_batch(input: &str) -> Result<String, String> {
    let mut pending = VecDeque::<PendingOutcome>::new();
    let mut responses: Vec<String> = Vec::new();

    for (line_number, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let op: Value = serde_json::from_str(line)
            .map_err(|err| format!("line {}: failed to parse JSON: {}", line_number + 1, err))?;
        let arr = op
            .as_array()
            .ok_or_else(|| format!("line {}: expected array operation", line_number + 1))?;

        let kind = arr
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("line {}: operation tag must be a string", line_number + 1))?;

        match kind {
            "push" => {
                let payload = arr.get(1).ok_or_else(|| {
                    format!("line {}: push operation missing payload", line_number + 1)
                })?;
                handle_push(payload, &mut pending).map_err(|err| {
                    format!("line {}: {}", line_number + 1, err)
                })?;
            }
            "pull" => {
                let import_id = arr
                    .get(1)
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| format!("line {}: pull expects numeric import id", line_number + 1))?;

                let outcome = pending.pop_front().unwrap_or_else(|| {
                    PendingOutcome::Error("no pending result for pull".to_string())
                });

                let message = match outcome {
                    PendingOutcome::Result(value) => json!(["result", import_id, value]),
                    PendingOutcome::Error(message) => json!([
                        "error",
                        import_id,
                        {
                            "message": message,
                        }
                    ]),
                };

                responses.push(
                    serde_json::to_string(&message)
                        .map_err(|err| format!("failed to serialize response: {}", err))?,
                );
            }
            other => {
                return Err(format!("line {}: unsupported operation `{}`", line_number + 1, other));
            }
        }
    }

    Ok(responses.join("\n"))
}

fn handle_push(payload: &Value, pending: &mut VecDeque<PendingOutcome>) -> Result<(), String> {
    let arr = payload
        .as_array()
        .ok_or_else(|| "push payload must be an array".to_string())?;

    let op_kind = arr
        .get(0)
        .and_then(|v| v.as_str())
        .ok_or_else(|| "push payload kind must be a string".to_string())?;

    match op_kind {
        "call" => {
            let cap_id = arr
                .get(1)
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "call operation missing numeric capability id".to_string())?;

            let path = arr
                .get(2)
                .and_then(|v| v.as_array())
                .ok_or_else(|| "call operation must include a method path array".to_string())?;

            let method = path
                .first()
                .and_then(|v| v.as_str())
                .ok_or_else(|| "call method name must be a string".to_string())?;

            let args: Vec<Value> = match arr.get(3) {
                Some(Value::Array(values)) => values.clone(),
                Some(_) => return Err("call arguments must be an array".to_string()),
                None => Vec::new(),
            };

            match cap_id {
                CALCULATOR_CAP_ID => {
                    match invoke_calculator(method, &args) {
                        Ok(value) => pending.push_back(PendingOutcome::Result(value)),
                        Err(err) => pending.push_back(PendingOutcome::Error(err)),
                    }
                }
                CHAT_CAP_ID => {
                    match invoke_chat(method, &args) {
                        Ok(value) => pending.push_back(PendingOutcome::Result(value)),
                        Err(err) => pending.push_back(PendingOutcome::Error(err)),
                    }
                }
                _ => {
                    pending.push_back(PendingOutcome::Error(format!(
                        "capability `{}` is not registered",
                        cap_id
                    )));
                }
            }
        }
        other => {
            pending.push_back(PendingOutcome::Error(format!(
                "unsupported push operation `{}`",
                other
            )));
        }
    }

    Ok(())
}

fn invoke_calculator(method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "add" => {
            if args.len() != 2 {
                return Err("`add` expects exactly two numeric arguments".into());
            }

            let a = args[0]
                .as_f64()
                .ok_or_else(|| "first argument must be a number".to_string())?;
            let b = args[1]
                .as_f64()
                .ok_or_else(|| "second argument must be a number".to_string())?;

            Ok(json!(a + b))
        }
        other => Err(format!("unknown calculator method `{}`", other)),
    }
}

fn invoke_chat(method: &str, args: &[Value]) -> Result<Value, String> {
    // This would need to be implemented with proper state management
    // For now, just return a placeholder
    match method {
        "auth" => {
            if args.len() != 2 {
                return Err("`auth` expects <username>, <password>".to_string());
            }
            
            let username = args[0]
                .as_str()
                .ok_or_else(|| "username must be a string".to_string())?;
            let password = args[1]
                .as_str()
                .ok_or_else(|| "password must be a string".to_string())?;

            // Simple credential validation
            let valid_credentials = [
                ("alice", "password123"),
                ("bob", "hunter2"),
                ("carol", "letmein"),
            ];
            
            let is_valid = valid_credentials.iter().any(|(u, p)| u == &username && p == &password);
            
            if is_valid {
                Ok(json!({
                    "session": {
                        "_type": "capability",
                        "id": 10000,
                    },
                    "user": username,
                }))
            } else {
                Err("Invalid credentials".to_string())
            }
        }
        "sendMessage" => {
            if args.len() != 1 {
                return Err("`sendMessage` expects <message>".to_string());
            }
            
            let message = args[0]
                .as_str()
                .ok_or_else(|| "message must be a string".to_string())?;

            // For now, just return success
            Ok(json!({
                "status": "ok",
                "echo": message,
            }))
        }
        "receiveMessages" => {
            // For now, return empty messages
            Ok(json!({
                "messages": []
            }))
        }
        "whoami" => {
            // For now, return a mock user
            Ok(json!({
                "user": "bob",
                "session": {
                    "_type": "capability",
                    "id": 10000,
                }
            }))
        }
        other => Err(format!("unknown chat method `{}`", other)),
    }
}
