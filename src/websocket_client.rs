use capnweb_core::CapId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};

const DEFAULT_BACKEND: &str = "ws://localhost:8787";
const CHAT_CAP_ID: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub from: String,
    pub body: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub method: String,
    pub args: Vec<Value>,
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub result: Option<Value>,
    pub error: Option<String>,
    pub id: u64,
}

// Local RPC target that the server can call (similar to ChatClient in TypeScript)
#[derive(Clone)]
pub struct ChatClient {
    pub on_message: Arc<Mutex<Option<Box<dyn Fn(ChatMessage) + Send + Sync>>>>,
}

impl ChatClient {
    pub fn new() -> Self {
        Self {
            on_message: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_on_message<F>(&self, callback: F)
    where
        F: Fn(ChatMessage) + Send + Sync + 'static,
    {
        let mut handler = self.on_message.lock().await;
        *handler = Some(Box::new(callback));
    }

    // This method will be called by the server via RPC
    pub async fn receive_message(&self, message: ChatMessage) {
        let handler = self.on_message.lock().await;
        if let Some(ref callback) = *handler {
            callback(message);
        } else {
            println!("{}: {}", message.from, message.body);
        }
    }
}

pub struct WebSocketClient {
    client: ChatClient,
    request_id: Arc<Mutex<u64>>,
    pending_requests: Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<RpcResponse>>>>,
    message_tx: mpsc::UnboundedSender<ChatMessage>,
    message_rx: Arc<Mutex<mpsc::UnboundedReceiver<ChatMessage>>>,
    request_tx: mpsc::UnboundedSender<Value>,
}

impl WebSocketClient {
    pub async fn new(url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let client = ChatClient::new();
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (request_tx, mut request_rx) = mpsc::unbounded_channel();
        
        let client = Self {
            client,
            request_id: Arc::new(Mutex::new(0)),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            message_tx,
            message_rx: Arc::new(Mutex::new(message_rx)),
            request_tx,
        };
        
        // Connect to WebSocket
        let (ws_stream, _) = connect_async(url).await?;
        let (mut ws_sink, mut ws_stream) = ws_stream.split();
        
        // Spawn task to handle incoming messages
        let request_id = client.request_id.clone();
        let pending_requests = client.pending_requests.clone();
        let local_client = client.client.clone();
        let message_tx = client.message_tx.clone();
        let request_tx_for_incoming = client.request_tx.clone();
        
        tokio::spawn(async move {
            while let Some(msg) = ws_stream.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(json_msg) = serde_json::from_str::<Value>(&text) {
                            // Handle Cap'n Web RPC responses
                            if let Some(array) = json_msg.as_array() {
                                if array.len() >= 2 {
                                    match array[0].as_str() {
                                        Some("resolve") => {
                                            // This is a resolve response: ["resolve", importId, value]
                                            if array.len() >= 3 {
                                                let import_id = array[1].as_u64().unwrap_or(0);
                                                let result = &array[2];
                                                let response = RpcResponse {
                                                    result: Some(result.clone()),
                                                    error: None,
                                                    id: import_id,
                                                };
                                                let mut pending = pending_requests.lock().await;
                                                if let Some(tx) = pending.remove(&import_id) {
                                                    let _ = tx.send(response);
                                                }
                                            }
                                        }
                                        Some("reject") => {
                                            // This is a reject response: ["reject", importId, error]
                                            if array.len() >= 3 {
                                                let import_id = array[1].as_u64().unwrap_or(0);
                                                let error_value = &array[2];
                                                let error_msg = if let Some(err_array) = error_value.as_array() {
                                                    if err_array.len() >= 2 {
                                                        err_array[1].as_str().unwrap_or("Unknown error")
                                                    } else {
                                                        "Unknown error"
                                                    }
                                                } else {
                                                    error_value.as_str().unwrap_or("Unknown error")
                                                };
                                                let response = RpcResponse {
                                                    result: None,
                                                    error: Some(error_msg.to_string()),
                                                    id: import_id,
                                                };
                                                let mut pending = pending_requests.lock().await;
                                                if let Some(tx) = pending.remove(&import_id) {
                                                    let _ = tx.send(response);
                                                }
                                            }
                                        }
                                        Some("push") => {
                                            // This is a server-initiated RPC call: ["push", ["pipeline", exportId, [method], [args]]]
                                            if array.len() >= 2 {
                                                if let Some(pipeline) = array[1].as_array() {
                                                    if pipeline.len() >= 4 && pipeline[0].as_str() == Some("pipeline") {
                                                        let method = pipeline[2].as_array().and_then(|m| m.get(0)).and_then(Value::as_str);
                                                        let args = pipeline[3].as_array();
                                                        
                                                        if let Some("receiveMessage") = method {
                                                            if let Some(args_array) = args {
                                                                if let Some(msg_data) = args_array.get(0) {
                                                                    if let Ok(chat_message) = serde_json::from_value::<ChatMessage>(msg_data.clone()) {
                                                                        local_client.receive_message(chat_message.clone()).await;
                                                                        let _ = message_tx.send(chat_message);
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Some("pull") => {
                                            // Server is requesting a value - we need to respond
                                            if array.len() >= 2 {
                                                let pull_id = array[1].as_u64().unwrap_or(0);
                                                // Respond with a resolve message: ["resolve", pullId, null]
                                                // The server is pulling the return value from a method call
                                                let resolve_msg = json!(["resolve", pull_id, null]);
                                                let _ = request_tx_for_incoming.send(resolve_msg);
                                            }
                                        }
                                        _ => {
                                            // Silently ignore unknown message types
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        break;
                    }
                    Err(e) => {
                        eprintln!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });
        
        // Spawn task to handle outgoing messages
        let request_tx_clone = client.request_tx.clone();
        tokio::spawn(async move {
            while let Some(request) = request_rx.recv().await {
                let message_text = serde_json::to_string(&request).unwrap_or_default();
                let message = Message::Text(message_text);
                if let Err(e) = ws_sink.send(message).await {
                    eprintln!("Failed to send WebSocket message: {}", e);
                    break;
                }
            }
        });
        
        Ok(client)
    }

    pub async fn call(&self, method: &str, args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        
        // Generate import ID (incremental)
        let import_id = {
            let mut request_id = self.request_id.lock().await;
            *request_id += 1;
            *request_id
        };

        // Store the response channel
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(import_id, tx);
        }

        // Send push message: ["push", ["pipeline", importId, [methodName], [args]]]
        // The main server capability is at import ID 0
        let push_msg = json!(["push", ["pipeline", 0, [method], args]]);
        self.request_tx.send(push_msg)?;

        // Send pull message: ["pull", importId]
        let pull_msg = json!(["pull", import_id]);
        self.request_tx.send(pull_msg)?;

        // Wait for response
        match rx.recv().await {
            Some(response) => {
                if let Some(error) = response.error {
                    return Err(error.into());
                }
                response.result.ok_or_else(|| "No result in response".into())
            }
            None => Err("Response channel closed".into()),
        }
    }

    pub async fn authenticate(&self, username: &str, password: &str) -> Result<CapId, Box<dyn std::error::Error + Send + Sync>> {
        let response = self.call("auth", vec![json!(username), json!(password)]).await?;
        
        let session_data = response.get("session")
            .ok_or("Authentication response missing session capability")?;
        
        let id_value = session_data.get("id")
            .and_then(Value::as_i64)
            .ok_or("Session capability missing id")?;
        
        let id = u64::try_from(id_value)
            .map_err(|_| "Session capability id must be non-negative")?;
        
        Ok(CapId::new(id))
    }

    pub async fn send_message(&self, capability: CapId, message: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.call("sendMessage", vec![json!(capability.as_u64()), json!(message)]).await?;
        Ok(())
    }

    pub async fn receive_messages(&self, capability: CapId) -> Result<Vec<ChatMessage>, Box<dyn std::error::Error + Send + Sync>> {
        let response = self.call("receiveMessages", vec![json!(capability.as_u64())]).await?;
        
        let messages = response.get("messages")
            .and_then(Value::as_array)
            .ok_or("Response missing messages array")?;
        
        let mut result = Vec::new();
        for msg in messages {
            if let Ok(chat_msg) = serde_json::from_value(msg.clone()) {
                result.push(chat_msg);
            }
        }
        
        Ok(result)
    }

    pub async fn whoami(&self, capability: CapId) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let response = self.call("whoami", vec![json!(capability.as_u64())]).await?;
        
        let username = response.get("username")
            .and_then(Value::as_str)
            .ok_or("Response missing username")?;
        
        Ok(username.to_string())
    }

    pub fn get_message_receiver(&self) -> Arc<Mutex<mpsc::UnboundedReceiver<ChatMessage>>> {
        self.message_rx.clone()
    }

    pub fn get_client(&self) -> &ChatClient {
        &self.client
    }
}

// Create WebSocket session similar to TypeScript newWebSocketRpcSession
pub async fn create_websocket_session(url: &str) -> Result<WebSocketClient, Box<dyn std::error::Error + Send + Sync>> {
    WebSocketClient::new(url).await
}