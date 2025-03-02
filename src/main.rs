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
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

    pub fn send_message(self, stream: &mut TcpStream) -> Result<()> {
        let serialized = self.serialize()?;
        send_serialized_message(stream, &serialized)?;
        Ok(())
    }

    pub fn receive_message(mut stream: &mut TcpStream) -> Result<Self> {
        let buffer = read_message_from_stream(stream)?;
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
            MessageType::Image(data) => {
                println!("Saving the image file");
                save_binary_content("images", None, data)?;
            }
            MessageType::File { name, content } => {
                save_binary_content("files", Some(name), content)?;
            }
        }
        Ok(())
    }
}

fn save_binary_content(dir: &str, filename: Option<&str>, data: &[u8]) -> Result<()> {
    fs::create_dir_all(dir)?;

    let path = match filename {
        Some(name) => format!("{}/{}", dir, name),
        None => {
            let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            format!("{}/{}.png", dir, timestamp)
        }
    };

    println!(
        "Receiving {}: {}",
        if filename.is_some() { "file" } else { "image" },
        path
    );
    let mut file = File::create(&path)?;
    file.write_all(data)?;
    println!("Saved to {}", path);

    Ok(())
}

fn read_message_from_stream(stream: &mut TcpStream) -> Result<Vec<u8>> {
    // Set read timeout to avoid hanging forever
    //stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    
    let mut len_bytes = [0u8; 4];

    // Read the message length
    // TODO consider rewritting using is_error or similar
    match stream.read_exact(&mut len_bytes) {
        Ok(_) => {},
        Err(e) => {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                return Err(anyhow::anyhow!("Connection closed by peer"));
            } else {
                return Err(anyhow::anyhow!("Failed to read message length: {}", e));
            }
        }
    }

    let len = u32::from_be_bytes(len_bytes) as usize;

    // Sanity check the length to avoid allocating too much memory
    if len == 0 {
        return Err(anyhow::anyhow!("Zero-length message received"));
    }
    
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
                // TODO consider removing
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut {
                    // Timeout, retry
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return Err(anyhow::anyhow!("Failed to read message data: {}", e));
            }
        }
    }

    Ok(buffer)
}

fn send_serialized_message(stream: &mut TcpStream, serialized: &[u8]) -> Result<()> {
    // Set write timeout
    //stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    
    // Send the length of the serialized message (as 4-byte value).
    let len = serialized.len() as u32;
    stream.write_all(&len.to_be_bytes())?;

    // Send the serialized message.
    stream.write_all(serialized)?;
    stream.flush()?;

    Ok(())
}

fn read_file_to_vec(path: &Path) -> Result<Vec<u8>> {
    let file =
        File::open(path).with_context(|| format!("Failed to open file: {}", path.display()))?;

    let mut buf_read = BufReader::new(file);
    let mut content = Vec::new();

    buf_read
        .read_to_end(&mut content)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    Ok(content)
}

fn get_filename_as_string(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or_default()
        .to_string()
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

                let content = read_file_to_vec(path).map_err(|_| ParseMessageError)?;
                let name = get_filename_as_string(path);

                Ok(MessageType::File { name, content })
            }
            s if s.starts_with(".image ") => {
                let path_str = s.strip_prefix(".image ").unwrap_or_default();
                let path = Path::new(path_str);

                let content = read_file_to_vec(path).map_err(|_| ParseMessageError)?;

                Ok(MessageType::Image(content))
            }
            s => Ok(MessageType::Text(s.to_string())),
        }
    }
}

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

fn handle_client(
    mut stream: TcpStream,
    client_addr: SocketAddr,
    clients: Arc<Mutex<HashMap<SocketAddr, TcpStream>>>,
) {
    // Send welcome message to confirm connection
    let welcome_msg = MessageType::Text(format!("Welcome to the chat server! Your address is {}", client_addr));
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
                // Log the received message
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

fn run_client(hostname: &str, port: u16) -> Result<()> {
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