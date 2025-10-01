use capnweb_core::{async_trait, CapId, RpcError};
use capnweb_server::{RpcTarget, Server, ServerConfig};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

const CALCULATOR_CAP_ID: u64 = 1;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::default();
    let server = Server::new(config);

    // Register capabilities
    server.register_capability(CapId::new(CALCULATOR_CAP_ID), Arc::new(Calculator::new()));

    // Run server with HTTP batch endpoint
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
