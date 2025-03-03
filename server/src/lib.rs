use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use utils::{MessageType, send_serialized_message};

fn forward_message_to_clients(
    message: &MessageType,
    sender_addr: SocketAddr,
    clients: &mut HashMap<SocketAddr, TcpStream>,
) {
    if let Ok(serialized) = message.serialize() {
        // Collect clients to remove if they're disconnected
        let mut to_remove = Vec::new();

        for (&addr, client_stream) in clients.iter_mut() {
            if addr != sender_addr {
                println!("Forwarding to {}", addr);
                if let Err(e) = send_serialized_message(client_stream, &serialized) {
                    println!("Error sending message to {}: {}", addr, e);
                    to_remove.push(addr);
                }
            }
        }

        // Remove disconnected clients
        for addr in to_remove {
            clients.remove(&addr);
            println!("Removed disconnected client: {}", addr);
        }
    }
}

fn handle_client(
    mut stream: TcpStream,
    client_addr: SocketAddr,
    clients: Arc<Mutex<HashMap<SocketAddr, TcpStream>>>,
) {
    // Send welcome message to confirm connection
    let welcome_msg = MessageType::Text(format!(
        "Welcome to the chat server! Your address is {}",
        client_addr
    ));
    if let Err(e) = welcome_msg.send_message(&mut stream) {
        println!("Failed to send welcome message to {}: {}", client_addr, e);
        // Remove client if we couldn't send a welcome message
        clients.lock().unwrap().remove(&client_addr);
        return;
    }

    loop {
        // Use a clone of the stream for receiving to avoid borrowing issues
        let mut stream_clone = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                println!("Error cloning stream for {}: {}", client_addr, e);
                break;
            }
        };

        match MessageType::receive_message(&mut stream_clone) {
            Ok(message) => {
                // Log the received message but don't save them
                match &message {
                    MessageType::Text(text) => println!("Received from {}: {}", client_addr, text),
                    MessageType::Image(_) => println!("Received image from {}", client_addr),
                    MessageType::File { name, .. } => {
                        println!("Received file '{}' from {}", name, client_addr)
                    }
                }

                // Forward to all other clients
                let mut client_map = clients.lock().unwrap();
                forward_message_to_clients(&message, client_addr, &mut client_map);
            }
            Err(e) => {
                println!("Error receiving from {}: {}", client_addr, e);
                break;
            }
        }
    }

    // Remove client when disconnected
    let mut clients_map = clients.lock().unwrap();
    clients_map.remove(&client_addr);
    println!("Client {} disconnected", client_addr);
}

pub fn listen_and_accept(hostname: &str, port: u16) -> Result<()> {
    let address = format!("{}:{}", hostname, port);
    let listener = TcpListener::bind(&address)?;
    println!("Server listening on {}", address);

    // Use Arc<Mutex<HashMap>> to safely share clients between threads
    let clients: Arc<Mutex<HashMap<SocketAddr, TcpStream>>> = Arc::new(Mutex::new(HashMap::new()));

    // Create directories for received files
    fs::create_dir_all("images")?;
    fs::create_dir_all("files")?;

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => match stream.peer_addr() {
                Ok(addr) => {
                    println!("New connection from {}", addr);

                    // Configure the stream
                    if let Err(e) = stream.set_nodelay(true) {
                        println!("Warning: Failed to set TCP_NODELAY: {}", e);
                    }

                    // Clone for the thread
                    let stream_clone = match stream.try_clone() {
                        Ok(s) => s,
                        Err(e) => {
                            println!("Error cloning stream: {}", e);
                            continue;
                        }
                    };

                    // Store client connection
                    clients.lock().unwrap().insert(addr, stream_clone);

                    // Clone Arc for the thread
                    let clients_clone = Arc::clone(&clients);

                    // Handle this client in a separate thread
                    thread::spawn(move || {
                        handle_client(stream, addr, clients_clone);
                    });
                }
                Err(e) => println!("Error getting peer address: {}", e),
            },
            Err(e) => println!("Error accepting connection: {}", e),
        }
    }

    Ok(())
}
