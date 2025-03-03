use anyhow::{Context, Result};
use std::io;
use std::net::TcpStream;
use std::str::FromStr;
use std::thread;

use utils::MessageType;

pub fn run_client(hostname: &str, port: u16) -> Result<()> {
    let address = format!("{}:{}", hostname, port);

    println!("Client connecting to {}", address);
    println!("Type messages to send (Ctrl+C to exit)");
    println!("  .file <path> - Send a file");
    println!("  .image <path> - Send an image");
    println!("  .quit - Exit the client");

    // Connect to the server - one connection for both sending and receiving
    let mut server_stream = TcpStream::connect(&address)
        .with_context(|| format!("Failed to connect to server at {}", address))?;

    // Configure socket options
    if let Err(e) = server_stream.set_nodelay(true) {
        println!("Warning: Failed to set TCP_NODELAY: {}", e);
    }

    println!("Connected to server!");

    // Clone the stream for the receiver thread
    let mut receiver_stream = server_stream.try_clone()?;

    // Start a thread to receive messages
    thread::spawn(move || {
        loop {
            // Receive and handle messages from the server
            match MessageType::receive_message(&mut receiver_stream) {
                Ok(message) => {
                    if let Err(e) = message.handle_received() {
                        println!("Error handling received message: {}", e);
                    }
                }
                Err(e) => {
                    println!("Error receiving message from server: {}", e);
                    println!("Connection to server lost. Exiting receiver thread.");
                    break;
                }
            }
        }
    });

    // Main loop for sending messages
    loop {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed_input = input.trim();

        if trimmed_input.is_empty() {
            continue;
        }

        if trimmed_input == ".quit" {
            println!("Shutting down the client");
            break;
        }

        let message = MessageType::from_str(trimmed_input);

        match message {
            Ok(msg) => {
                if let Err(e) = msg.send_message(&mut server_stream) {
                    println!("Failed to send message: {}", e);
                    println!("Connection to server might be lost. Try again or quit.");
                }
            }
            Err(_) => println!("Failed to parse input: {}", trimmed_input),
        }
    }

    Ok(())
}
