use anyhow::{Context, Result};
use std::io;
use std::net::TcpStream;
use std::str::FromStr;
use std::thread;
use tracing::{info, warn, error, debug};

use utils::MessageType;

pub fn run_client(hostname: &str, port: u16) -> Result<()> {
    let address = format!("{}:{}", hostname, port);

    info!("Client connecting to {}", address);
    info!("Type messages to send (Ctrl+C to exit)");
    info!("  .file <path> - Send a file");
    info!("  .image <path> - Send an image");
    info!("  .quit - Exit the client");

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
    let mut receiver_stream = server_stream.try_clone()
        .context("Failed to clone TCP stream for receiver thread")?;

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
                        error!("Error handling received message: {}", e);
                    }
                }
                Err(e) => {
                    error!("Error receiving message from server: {}", e);
                    info!("Connection to server lost. Exiting receiver thread.");
                    break;
                }
            }
        }
        info!("Receiver thread terminated");
    });

    // Main loop for sending messages
    info!("Starting main message loop, enter messages to send");
    loop {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed_input = input.trim();

        if trimmed_input.is_empty() {
            debug!("Empty input, skipping");
            continue;
        }

        if trimmed_input == ".quit" {
            info!("Shutting down the client");
            break;
        }

        debug!("Parsing input: '{}'", trimmed_input);
        let message = MessageType::from_str(trimmed_input);

        match message {
            Ok(msg) => {
                debug!("Successfully parsed message: {:?}", msg);
                info!("Sending message to server");
                if let Err(e) = msg.send_message(&mut server_stream) {
                    error!("Failed to send message: {}", e);
                    warn!("Connection to server might be lost. Try again or quit.");
                } else {
                    debug!("Message sent successfully");
                }
            }
            Err(_) => error!("Failed to parse input: {}", trimmed_input),
        }
    }

    debug!("Client main loop exited, returning normally");
    Ok(())
}
