use anyhow::{Context, Result as AnyhowResult};
use std::collections::HashMap;
use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{debug, error, info, trace, warn};
use utils::errors::{ChatError, ChatResult};

use utils::{MessageType, error_codes, send_serialized_message};

fn forward_message_to_clients(
    message: &MessageType,
    sender_addr: SocketAddr,
    clients: &mut HashMap<SocketAddr, TcpStream>,
) -> ChatResult<()> {
    let serialized = message.serialize().map_err(|e| {
        ChatError::SerializationError(format!("Failed to serialize message for forwarding: {}", e))
    })?;

    // Collect clients to remove if they're disconnected
    let mut to_remove = Vec::new();
    let client_count = clients.len() - 1; // Exclude sender

    debug!(
        "Forwarding message from {} to {} clients",
        sender_addr, client_count
    );

    for (&addr, client_stream) in clients.iter_mut() {
        if addr != sender_addr {
            debug!("Forwarding to {}", addr);
            if let Err(e) = send_serialized_message(client_stream, &serialized) {
                error!("Error sending message to {}: {}", addr, e);
                to_remove.push(addr);
            }
        }
    }

    // Remove disconnected clients
    for addr in to_remove {
        clients.remove(&addr);
        info!("Removed disconnected client: {}", addr);
    }

    Ok(())
}

fn handle_client(
    mut stream: TcpStream,
    client_addr: SocketAddr,
    clients: Arc<Mutex<HashMap<SocketAddr, TcpStream>>>,
) {
    info!("Starting handler for client: {}", client_addr);

    // Send welcome message to confirm connection
    let welcome_msg = MessageType::Text(format!(
        "Welcome to the chat server! Your address is {}",
        client_addr
    ));

    debug!("Sending welcome message to {}", client_addr);
    if let Err(e) = welcome_msg.send_message(&mut stream) {
        error!("Failed to send welcome message to {}: {}", client_addr, e);
        // Remove client if we couldn't send a welcome message
        if let Ok(mut clients_map) = clients.lock() {
            clients_map.remove(&client_addr);
        }
        return;
    }

    loop {
        // Use a clone of the stream for receiving to avoid borrowing issues
        let mut stream_clone = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                error!("Error cloning stream for {}: {}", client_addr, e);
                // Send error to client before breaking
                let _ = MessageType::create_error(
                    error_codes::SERVER_ERROR,
                    "Internal server error: Failed to process your connection",
                )
                .send_message(&mut stream);
                break;
            }
        };

        trace!("Waiting for message from client {}", client_addr);
        match MessageType::receive_message(&mut stream_clone) {
            Ok(message) => {
                // Log the received message but don't save them
                match &message {
                    MessageType::Text(text) => info!("Received from {}: {}", client_addr, text),
                    MessageType::Image(data) => {
                        info!("Received image ({} bytes) from {}", data.len(), client_addr)
                    }
                    MessageType::File { name, content } => {
                        info!(
                            "Received file '{}' ({} bytes) from {}",
                            name,
                            content.len(),
                            client_addr
                        )
                    }
                    MessageType::Error { code, message } => {
                        info!(
                            "Received error from {}: code={}, message={}",
                            client_addr, code, message
                        )
                    }
                }

                // Forward to all other clients
                let client_map_result = clients.lock();
                match client_map_result {
                    Ok(mut client_map) => {
                        if let Err(e) =
                            forward_message_to_clients(&message, client_addr, &mut client_map)
                        {
                            error!("Failed to forward message from {}: {}", client_addr, e);
                            // Send error back to the sender
                            let _ = MessageType::create_error(
                                error_codes::SERVER_ERROR,
                                &format!("Failed to forward your message: {}", e),
                            )
                            .send_message(&mut stream);
                        }
                    }
                    Err(e) => {
                        error!("Failed to acquire lock on clients map: {}", e);
                        let _ = MessageType::create_error(
                            error_codes::SERVER_ERROR,
                            "Internal server error: Failed to process your message",
                        )
                        .send_message(&mut stream);
                        continue;
                    }
                }
            }
            Err(e) => {
                warn!("Error receiving from {}: {}", client_addr, e);

                // For some errors, don't break the connection
                match &e {
                    ChatError::EmptyMessage => {
                        let _ = MessageType::create_error(
                            error_codes::MESSAGE_ERROR,
                            "Empty message received",
                        )
                        .send_message(&mut stream);
                        continue;
                    }
                    ChatError::MessageTooLarge(size) => {
                        let _ = MessageType::create_error(
                            error_codes::MESSAGE_ERROR,
                            &format!("Message too large: {} bytes (max: 100MB)", size),
                        )
                        .send_message(&mut stream);
                        continue;
                    }
                    ChatError::SerializationError(msg) => {
                        let _ = MessageType::create_error(
                            error_codes::PROTOCOL_ERROR,
                            &format!("Invalid message format: {}", msg),
                        )
                        .send_message(&mut stream);
                        continue;
                    }
                    _ => break, // For connection errors, break the loop
                }
            }
        }
    }

    // Remove client when disconnected
    match clients.lock() {
        Ok(mut clients_map) => {
            clients_map.remove(&client_addr);
            info!("Client {} disconnected", client_addr);
        }
        Err(e) => error!(
            "Failed to acquire lock to remove client {}: {}",
            client_addr, e
        ),
    }
}

pub fn listen_and_accept(hostname: &str, port: u16) -> AnyhowResult<()> {
    let address = format!("{}:{}", hostname, port);
    info!("Starting server on {}", address);

    let listener =
        TcpListener::bind(&address).with_context(|| format!("Failed to bind to {}", address))?;

    info!("Server listening on {}", address);

    // Use Arc<Mutex<HashMap>> to safely share clients between threads
    let clients: Arc<Mutex<HashMap<SocketAddr, TcpStream>>> = Arc::new(Mutex::new(HashMap::new()));

    // Create directories for received files
    debug!("Creating directories for received content");
    fs::create_dir_all("images").with_context(|| "Failed to create 'images' directory")?;

    fs::create_dir_all("files").with_context(|| "Failed to create 'files' directory")?;

    info!("Server ready to accept connections");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => match stream.peer_addr() {
                Ok(addr) => {
                    info!("New connection from {}", addr);

                    // Configure the stream
                    if let Err(e) = stream.set_nodelay(true) {
                        warn!("Failed to set TCP_NODELAY for {}: {}", addr, e);
                    }

                    // Clone for the thread
                    let stream_clone = match stream.try_clone() {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Error cloning stream for {}: {}", addr, e);
                            continue;
                        }
                    };

                    // Store client connection
                    match clients.lock() {
                        Ok(mut map) => {
                            map.insert(addr, stream_clone);
                            debug!("Added client {} to active connections map", addr);
                        }
                        Err(e) => {
                            error!("Failed to acquire lock to add client {}: {}", addr, e);
                            continue;
                        }
                    }

                    // Clone Arc for the thread
                    let clients_clone = Arc::clone(&clients);

                    // Handle this client in a separate thread
                    debug!("Spawning handler thread for client {}", addr);
                    thread::spawn(move || {
                        handle_client(stream, addr, clients_clone);
                    });
                }
                Err(e) => error!("Error getting peer address: {}", e),
            },
            Err(e) => error!("Error accepting connection: {}", e),
        }
    }

    warn!("Server listener loop exited");
    Ok(())
}
