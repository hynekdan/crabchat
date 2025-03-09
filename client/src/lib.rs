use anyhow::{Context, Result as AnyhowResult};
use std::io::{self, Write};
use std::net::TcpStream;
use std::str::FromStr;
use std::thread;
use tracing::{debug, error, info, warn};
use utils::errors::ChatError;

use utils::MessageType;

pub fn run_client(hostname: &str, port: u16) -> AnyhowResult<()> {
    let address = format!("{}:{}", hostname, port);

    info!("Client connecting to {}", address);
    print_usage_instructions();

    // Connect to the server - one connection for both sending and receiving
    debug!("Attempting TCP connection to {}", address);
    let mut server_stream = TcpStream::connect(&address)
        .with_context(|| format!("Failed to connect to server at {}", address))?;

    // Configure socket options
    if let Err(e) = server_stream.set_nodelay(true) {
        warn!("Failed to set TCP_NODELAY: {}", e);
    }

    info!("Connected to server!");

    // Clone the stream for the receiver thread
    let mut receiver_stream = server_stream
        .try_clone()
        .with_context(|| "Failed to clone TCP stream for receiver thread")?;

    // Start a thread to receive messages
    debug!("Spawning receiver thread");
    thread::spawn(move || {
        info!("Message receiver thread started");
        loop {
            // Receive and handle messages from the server
            debug!("Waiting for incoming message from server");
            match MessageType::receive_message(&mut receiver_stream) {
                Ok(message) => {
                    debug!("Received message from server: {:?}", message);
                    if let Err(e) = message.handle_received() {
                        error!("Error handling received message: {:#}", e);
                        let chat_error: ChatError = e.into();
                        chat_error.display_error_message();
                    }
                }
                Err(e) => {
                    error!("Error receiving message from server: {}", e);
                    e.display_error_message();

                    if matches!(e, ChatError::ConnectionClosed) {
                        info!("Connection to server lost. Exiting receiver thread.");
                        info!("Connection to server lost. You may continue typing, but messages won't be sent.");
                        info!("Please restart the client to reconnect.");
                        break;
                    }
                }
            }
        }
        info!("Receiver thread terminated");
    });

    // Main loop for sending messages
    info!("Starting main message loop, enter messages to send");
    loop {
        info!("> ");
        io::stdout()
            .flush()
            .with_context(|| "Failed to flush stdout")?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .with_context(|| "Failed to read input")?;

        let trimmed_input = input.trim();

        if trimmed_input.is_empty() {
            debug!("Empty input, skipping");
            continue;
        }

        if trimmed_input == ".quit" {
            info!("Shutting down the client");
            break;
        }

        if trimmed_input == ".help" {
            print_usage_instructions();
            continue;
        }

        debug!("Parsing input: '{}'", trimmed_input);
        let message = MessageType::from_str(trimmed_input);

        match message {
            Ok(msg) => {
                debug!("Successfully parsed message: {:?}", msg);
                info!("Sending message to server");
                if let Err(e) = msg.send_message(&mut server_stream) {
                    error!("Failed to send message: {:#}", e);

                    // Check if the error is a connection error - need to convert to ChatError to check
                    let chat_error: ChatError = e.into();
                    chat_error.display_error_message();

                    if matches!(chat_error, ChatError::ConnectionClosed) {
                        error!(
                            "Connection to server lost. Please restart the client to reconnect."
                        );
                        break;
                    } else {
                        warn!("Connection to server might be unstable. Try again or quit.");
                    }
                } else {
                    debug!("Message sent successfully");
                }
            }
            Err(_) => {
                error!("Failed to parse input: {}", trimmed_input);
                error!("Invalid input. Type .help for usage instructions.");
            }
        }
    }

    debug!("Client main loop exited, returning normally");
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
