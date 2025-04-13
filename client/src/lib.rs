//! Library module for the CrabChat client application.
//!
//! This module contains the core logic for the client, including connecting to the server,
//! handling user input, sending and receiving messages, and managing files and images.

use anyhow::{Context, Result as AnyhowResult, anyhow};
use std::path::Path;
use tokio::io::{self as tokio_io, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use utils::errors::ChatError;
use utils::{LocalCommandType, MessageType, get_filename_as_string, parse_input, read_file_to_vec};

/// Runs the CrabChat client.
///
/// This function connects to the server, handles user input, and processes incoming messages.
///
/// # Arguments
/// - `hostname`: The server's hostname or IP address.
/// - `port`: The server's port number.
/// - `username`: The username to use for the client.
///
/// # Errors
/// Returns an error if the client fails to connect to the server, login, or handle messages.
pub async fn run_client(hostname: &str, port: u16, username: Option<String>) -> AnyhowResult<()> {
    let address = format!("{}:{}", hostname, port);
    let user = username.ok_or_else(|| anyhow!("Username is required (.e.g., -u Daniel)"))?;

    info!("Client '{}' connecting to {}", user, address);
    print_usage_instructions();

    // Connect to the server - one connection for both sending and receiving
    debug!("Attempting TCP connection to {}", address);
    let server_stream = TcpStream::connect(&address)
        .await
        .with_context(|| format!("Failed to connect to server at {}", address))?;

    // Configure socket options
    if let Err(e) = server_stream.set_nodelay(true) {
        warn!("Failed to set TCP_NODELAY: {}", e);
    }

    info!("Connected to server! Attempting login...");

    // Split the stream into reader and writer
    let (reader, mut writer) = tokio_io::split(server_stream);
    let mut buf_reader = BufReader::new(reader);

    // Login - currently without password
    let login_msg = MessageType::Login(user.clone());
    login_msg
        .send(&mut writer)
        .await
        .context("Failed to send login message")?;
    debug!("Login message sent for user '{}'", user);

    // Wait for LoginOk or Error
    match MessageType::receive(&mut buf_reader).await {
        Ok(MessageType::LoginOk) => {
            info!("Login successful!");
        }
        Ok(MessageType::Error(err_msg)) => {
            error!("Login failed: {}", err_msg);
            return Err(anyhow!("Server rejected login: {}", err_msg));
        }
        Ok(other) => {
            error!(
                "Unexpected message received after login attempt: {:?}",
                other
            );
            return Err(anyhow!("Unexpected response from server during login"));
        }
        Err(e) => {
            error!("Failed to receive login response: {}", e);
            return Err(e).context("Connection error during login");
        }
    }

    // Channel for signaling shutdown from input task to receiver task
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

    // Receiver Task
    let save_path = Path::new("client_received").to_path_buf();
    tokio::spawn(async move {
        info!("Message receiver task started.");
        loop {
            tokio::select! {
                // Wait for incoming message or shutdown signal
                result = MessageType::receive(&mut buf_reader) => {
                    match result {
                        Ok(message) => {
                            debug!("Received message from server: {:?}", message);
                            if let Err(e) = message.handle_received_client(&save_path).await {
                                error!("Error handling received message: {:#}", e);
                            }
                        }
                        Err(ChatError::ConnectionClosed) => {
                            info!("Connection closed by server. Receiver task exiting.");
                            break;
                        }
                        Err(e) => {
                            error!("Error receiving message from server: {}", e);
                            // If there is a connection error, exit the loop
                            if matches!(e, ChatError::ConnectionError(_)) {
                                break;
                            }
                        }
                    }
                },
                 _ = shutdown_rx.recv() => {
                    info!("Shutdown signal received. Receiver task exiting.");
                    break;
                }
            }
        }
        info!("Receiver task terminated");
    });

    // Main loop for sending messages
    info!("Enter messages, .file <path>, .image <path>, .help, or .quit");
    let mut stdin_reader = BufReader::new(tokio_io::stdin());
    let mut input_buf = String::new();

    loop {
        input_buf.clear();
        info!("> ");
        tokio_io::stdout()
            .flush()
            .await
            .context("Failed to flush stdout")?;

        // Read line async
        match stdin_reader.read_line(&mut input_buf).await {
            Ok(0) => {
                info!("Stdin closed. Shutting down.");
                break; // EOF
            }
            Ok(_) => {
                // Parse the input
                match parse_input(&input_buf) {
                    Ok(MessageType::LocalCommand(cmd)) => match cmd {
                        LocalCommandType::Quit => {
                            info!("Quit command received. Shutting down.");
                            break;
                        }
                        LocalCommandType::Help => {
                            print_usage_instructions();
                            continue;
                        }
                        LocalCommandType::SendFile(path) => match read_file_to_vec(&path).await {
                            Ok(content) => {
                                let name = get_filename_as_string(&path);
                                info!("Sending file '{}' ({} bytes)", name, content.len());
                                let msg = MessageType::File { name, content };
                                if let Err(e) = msg.send(&mut writer).await {
                                    error!("Failed to send file message: {}", e);
                                    if matches!(
                                        e,
                                        ChatError::ConnectionClosed | ChatError::ConnectionError(_)
                                    ) {
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Failed to read file {}: {}", path.display(), e)
                            }
                        },
                        LocalCommandType::SendImage(path) => match read_file_to_vec(&path).await {
                            Ok(content) => {
                                info!(
                                    "Sending image '{}' ({} bytes)",
                                    path.display(),
                                    content.len()
                                );
                                let msg = MessageType::Image(content);
                                if let Err(e) = msg.send(&mut writer).await {
                                    error!("Failed to send image message: {}", e);
                                    if matches!(
                                        e,
                                        ChatError::ConnectionClosed | ChatError::ConnectionError(_)
                                    ) {
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Failed to read image {}: {}", path.display(), e)
                            }
                        },
                    },
                    Ok(msg @ MessageType::Text(_)) => {
                        debug!("Sending text message");
                        if let Err(e) = msg.send(&mut writer).await {
                            error!("Failed to send text message: {}", e);
                            if matches!(
                                e,
                                ChatError::ConnectionClosed | ChatError::ConnectionError(_)
                            ) {
                                break;
                            }
                        }
                    }
                    Ok(_) => {
                        warn!("Unexpected message type parsed from input: {:?}", input_buf);
                    }
                    Err(e) => {
                        error!("Input error: {}", e);
                        if e == "Input is empty" {
                            continue;
                        }
                        print_usage_instructions();
                    }
                }
            }
            Err(e) => {
                error!("Failed to read stdin: {}", e);
                break;
            }
        }
    }

    let _ = shutdown_tx.send(()).await; // Ignore error if receiver already closed

    info!("Client shutting down gracefully.");
    Ok(())
}

fn print_usage_instructions() {
    info!("=== CrabChat Client Commands ===");
    info!("  Just type to send text messages");
    info!("  .file <path> - Send a file");
    info!("  .image <path> - Send an image");
    info!("  .help - Show this help message");
    info!("  .quit - Exit the client");
    info!("==============================");
}
