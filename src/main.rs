use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_cbor::Result as CborResult;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, io};

#[derive(Serialize, Deserialize, Debug, Clone)]
enum MessageType {
    Text(String),
    Image(Vec<u8>),
    File { name: String, content: Vec<u8> },
}

impl MessageType {
    fn serialize(&self) -> CborResult<Vec<u8>> {
        serde_cbor::to_vec(&self)
    }

    fn deserialize(input: &[u8]) -> CborResult<Self> {
        serde_cbor::from_slice(input)
    }

    pub fn send_message(self, address: &str) -> Result<()> {
        let serialized = self.serialize()?;
        let mut stream = TcpStream::connect(address)?;

        // Send the length of the serialized message (as 4-byte value).
        let len = serialized.len() as u32;
        stream.write_all(&len.to_be_bytes())?;

        // Send the serialized message.
        stream.write_all(&serialized)?;
        stream.flush()?;

        Ok(())
    }

    pub fn receive_message(mut stream: TcpStream) -> Result<Self> {
        let mut len_bytes = [0u8; 4];

        // Read the message length with more robust handling
        // TODO consider rewritting using is_error or similar
        match stream.read_exact(&mut len_bytes) {
            Ok(_) => {}
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to read message length: {}", e));
            }
        }

        let len = u32::from_be_bytes(len_bytes) as usize;

        // Sanity check the length to avoid allocating too much memory
        if len > 100_000_000 {
            // 100MB limit
            return Err(anyhow::anyhow!("Message too large: {} bytes", len));
        }

        let mut buffer = vec![0u8; len];

        // Read the full message with better error handling
        let mut bytes_read = 0;
        while bytes_read < len {
            match stream.read(&mut buffer[bytes_read..]) {
                Ok(0) => {
                    return Err(anyhow::anyhow!(
                        "Connection closed before reading full message"
                    ));
                }
                Ok(n) => bytes_read += n,
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to read message data: {}", e));
                }
            }
        }

        match Self::deserialize(&buffer) {
            Ok(message) => Ok(message),
            Err(e) => Err(anyhow::anyhow!("Failed to deserialize message: {}", e)),
        }
    }

    pub fn handle_received(&self) -> Result<()> {
        match self {
            MessageType::Text(text) => {
                println!("Received: {}", text);
            }
            // TODO merge image and file handling
            MessageType::Image(data) => {
                println!("Receiving image...");
                fs::create_dir("images")?;

                let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

                let path = format!("images/{}.png", timestamp);
                let mut file = File::create(&path)?;
                file.write_all(data)?;
                println!("Image saved to {}", path);
            }
            MessageType::File { name, content } => {
                println!("Receiving file: {}", name);
                fs::create_dir("files")?;

                let path = format!("files/{}", name);
                let mut file = File::create(&path)?;
                file.write_all(content)?;
                println!("File saved to {}", path);
            }
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParseMessageError;

impl FromStr for MessageType {
    type Err = ParseMessageError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            s if s.starts_with(".file ") => {
                // is it possible to use something like .unwrap_or(unreachable!())?
                let path_str = s.strip_prefix(".file ").unwrap_or_default();
                let path = Path::new(path_str);

                let file = match File::open(path) {
                    Ok(f) => f,
                    Err(_) => return Err(ParseMessageError),
                };

                let mut buf_read = BufReader::new(file);
                let mut content: Vec<u8> = Vec::new();

                if buf_read.read_to_end(&mut content).is_err() {
                    return Err(ParseMessageError);
                }

                // using unwrap as I already read the file
                let file_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or_default();

                Ok(MessageType::File {
                    name: file_name.to_string(),
                    content,
                })
            }
            s if s.starts_with(".image ") => {
                let path_str = s.strip_prefix(".image ").unwrap_or_default();
                let path = Path::new(path_str);

                let file = match File::open(path) {
                    Ok(f) => f,
                    Err(_) => return Err(ParseMessageError),
                };

                let mut buf_read = BufReader::new(file);
                let mut content: Vec<u8> = Vec::new();

                if buf_read.read_to_end(&mut content).is_err() {
                    return Err(ParseMessageError);
                }

                Ok(MessageType::Image(content))
            }
            s => Ok(MessageType::Text(s.to_string())),
        }
    }
}

fn listen_and_accept(hostname: &str, port: u16) -> Result<()> {
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

fn handle_client(
    stream: TcpStream,
    client_addr: SocketAddr,
    clients: Arc<Mutex<HashMap<SocketAddr, TcpStream>>>,
) {
    loop {
        let stream_clone = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                println!("Error cloning stream for {}: {}", client_addr, e);
                break;
            }
        };

        match MessageType::receive_message(stream_clone) {
            Ok(message) => {
                // Log the received message
                match &message {
                    MessageType::Text(text) => println!("Received from {}: {}", client_addr, text),
                    MessageType::Image(_) => println!("Received image from {}", client_addr),
                    MessageType::File { name, .. } => {
                        println!("Received file '{}' from {}", name, client_addr)
                    }
                }

                println!("Got the message properly");

                // Forward to all other clients
                let client_map = clients.lock().unwrap();
                for (&addr, client_stream) in client_map.iter() {
                    if addr != client_addr {
                        println!("Forwarding to {}", addr);
                        if let Ok(mut stream) = client_stream.try_clone() {
                            // Send message length
                            if let Ok(serialized) = message.serialize() {
                                let len = serialized.len() as u32;
                                if let Err(e) = stream.write_all(&len.to_be_bytes()) {
                                    println!("Error sending message length to {}: {}", addr, e);
                                    continue;
                                }

                                // Send the serialized message
                                if let Err(e) = stream.write_all(&serialized) {
                                    println!("Error sending message data to {}: {}", addr, e);
                                    continue;
                                }

                                if let Err(e) = stream.flush() {
                                    println!("Error flushing stream to {}: {}", addr, e);
                                }
                            }
                        }
                    }
                }
                println!("Forward finished");
            }
            Err(e) => {
                println!("Error receiving from {}: {}", client_addr, e);
                break;
            }
        }
    }

    // Remove client when disconnected
    clients.lock().unwrap().remove(&client_addr);
    println!("Client {} disconnected", client_addr);
}

fn run_client(hostname: &str, port: u16) -> Result<()> {
    let address = format!("{}:{}", hostname, port);

    println!("Client connecting to {}", address);
    println!("Type messages to send (Ctrl+C to exit)");
    println!("  .file <path> - Send a file");
    println!("  .image <path> - Send an image");
    println!("  .quit - Exit the client");

    // Start a thread to receive messages
    let recv_address = address.clone();
    thread::spawn(move || {
        // Try to create a listener socket for receiving messages
        if let Ok(listener) = TcpListener::bind("127.0.0.1:0") {
            // Get our local address that we're listening on
            let local_addr = listener.local_addr().unwrap();

            // Connect to the server to register our address
            if let Ok(mut server) = TcpStream::connect(&recv_address) {
                // Register with server
                let register_msg = MessageType::Text(format!("REGISTER:{}", local_addr));
                if let Ok(serialized) = register_msg.serialize() {
                    let len = serialized.len() as u32;
                    let _ = server.write_all(&len.to_be_bytes());
                    let _ = server.write_all(&serialized);
                }

                // Listen for incoming messages
                for stream in listener.incoming().flatten() {
                    if let Ok(message) = MessageType::receive_message(stream) {
                        let _ = message.handle_received();
                    }
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
                if let Err(e) = msg.send_message(&address) {
                    println!("Failed to send message: {}", e);
                }
            }
            Err(_) => println!("Failed to parse input: {}", trimmed_input),
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    // Default settings
    let mut mode = "client";
    let mut hostname = "localhost";
    let mut port = 11111;

    // Parse command line arguments
    if args.len() > 1 && args[1] == "server" {
        mode = "server";
    }

    // Check for hostname and port args
    for i in 1..args.len() {
        if (args[i] == "--hostname" || args[i] == "-h") && i + 1 < args.len() {
            hostname = &args[i + 1];
        } else if (args[i] == "--port" || args[i] == "-p") && i + 1 < args.len() {
            if let Ok(p) = args[i + 1].parse::<u16>() {
                port = p;
            }
        }
    }

    if mode == "server" {
        listen_and_accept(hostname, port).context("Failed to run server")?;
    } else {
        run_client(hostname, port).context("Failed to run client")?;
    }

    Ok(())
}
