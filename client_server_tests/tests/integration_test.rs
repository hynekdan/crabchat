use client::run_client;
use server::ServerConfig;
use server::listen_and_accept;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use utils::MessageType;

#[tokio::test]
async fn test_client_server_integration() {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        let hostname = "127.0.0.1";
        let port = 12345;
        let admin_hostname = "127.0.0.1";
        let admin_port = 12346;

        let server_config = ServerConfig {
            hostname: hostname.to_string(),
            port,
            admin_hostname: admin_hostname.to_string(),
            admin_port,
            metrics_hostname: hostname.to_string(),
            metrics_port: 12347,
        };

        let server_future = listen_and_accept(server_config);
        tokio::select! {
            _ = server_future => {},
            _ = shutdown_rx => {
                println!("Server shutting down...");
            }
        }
    });

    // Give the server some time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let client_task = tokio::spawn(async move {
        let hostname = "127.0.0.1".to_string();
        let port = 12345;
        let username = Some("test_user".to_string());

        let result = run_client(&hostname, port, username).await;
        assert!(result.is_ok(), "Client failed to run: {:?}", result.err());
    });

    // Simulate a client sending a message
    let client_message_task = tokio::spawn(async move {
        let mut client_stream = TcpStream::connect("127.0.0.1:12345").await.unwrap();

        let message = MessageType::Text("Hello, CrabChat!".to_string());
        message.send(&mut client_stream).await.unwrap();

        let response = MessageType::receive(&mut client_stream).await.unwrap();
        match response {
            MessageType::Text(text) => {
                assert_eq!(text, "Hello, CrabChat!");
            }
            _ => panic!("Unexpected response from server"),
        }
    });

    let _ = tokio::join!(client_task, client_message_task);

    let _ = shutdown_tx.send(());
    let _ = server_task.await;
}
