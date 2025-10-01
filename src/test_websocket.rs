use tokio_tungstenite::{connect_async, accept_async};
use tokio::net::TcpListener;
use futures_util::{SinkExt, StreamExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start server
    let server = tokio::spawn(async {
        let listener = TcpListener::bind("127.0.0.1:8081").await?;
        println!("Test WebSocket server listening on 127.0.0.1:8081");
        
        while let Ok((stream, addr)) = listener.accept().await {
            println!("New connection from: {}", addr);
            let ws_stream = accept_async(stream).await?;
            let (mut ws_sender, mut ws_receiver) = ws_stream.split();
            
            tokio::spawn(async move {
                while let Some(msg) = ws_receiver.next().await {
                    match msg {
                        Ok(msg) => {
                            println!("Received: {:?}", msg);
                            let _ = ws_sender.send(msg).await;
                        }
                        Err(e) => {
                            println!("Error: {}", e);
                            break;
                        }
                    }
                }
            });
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    // Start client
    let client = tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        let (ws_stream, _) = connect_async("ws://127.0.0.1:8081").await?;
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();
        
        // Send a test message
        let _ = ws_sender.send(tokio_tungstenite::tungstenite::Message::Text("Hello Server!".to_string())).await;
        
        // Receive response
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(msg) => {
                    println!("Client received: {:?}", msg);
                    break;
                }
                Err(e) => {
                    println!("Client error: {}", e);
                    break;
                }
            }
        }
        
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    // Wait for both
    let _ = tokio::try_join!(server, client)?;
    
    Ok(())
}
