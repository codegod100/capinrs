use capnweb_client::{Client, ClientConfig};
use capnweb_core::CapId;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client configuration
    let config = ClientConfig {
        url: "http://localhost:8080/rpc/batch".to_string(),
        ..Default::default()
    };
    let client = Client::new(config)?;

    // Make RPC calls
    let result = client
        .call(CapId::new(1), "add", vec![json!(10), json!(20)])
        .await?;
    println!("Result: {}", result);

    Ok(())
}
