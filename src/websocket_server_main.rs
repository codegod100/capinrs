mod websocket_server;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use websocket_server::WebSocketServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:8080";
    let listener = TcpListener::bind(addr).await?;
    println!("WebSocket server listening on {}", addr);

    let server = WebSocketServer::new();

    while let Ok((stream, addr)) = listener.accept().await {
        println!("New connection from: {}", addr);

        let ws_stream = accept_async(stream).await?;
        let server_clone = server.clone();

        tokio::spawn(async move {
            server_clone.handle_websocket(ws_stream).await;
        });
    }

    Ok(())
}
