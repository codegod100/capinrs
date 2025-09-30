use capnweb_core::{CapId, RpcError, async_trait};
use capnweb_server::{RpcTarget, Server, ServerConfig};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug)]
struct Calculator;

#[async_trait]
impl RpcTarget for Calculator {
    async fn call(&self, member: &str, args: Vec<Value>) -> Result<Value, RpcError> {
        match member {
            "add" => {
                let (a, b) = expect_two_numbers(member, &args)?;
                Ok(json!(a + b))
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
    server.register_capability(CapId::new(1), Arc::new(Calculator));

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
