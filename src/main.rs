use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_cbor::Result as CborResult;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::str::FromStr;
use std::{env, fs};

#[derive(Serialize, Deserialize, Debug)]
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
        stream.read_exact(&mut len_bytes)?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        let mut buffer = vec![0u8; len];
        stream.read_exact(&mut buffer)?;

        Ok(Self::deserialize(&buffer)?)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ParseMessageError;

impl FromStr for MessageType {
    type Err = ParseMessageError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            s if s.starts_with(".file ") => {
                let path = Path::new(s.strip_prefix(".file ").unwrap_or_default());
                let mut buf_read = BufReader::new(File::open(path).unwrap());
                let mut content: Vec<u8> = Vec::new();
                let mut buffer = [0u8; 1024];

                loop {
                    match buf_read.read(&mut buffer) {
                        Ok(0) => break, // End of file
                        Ok(n) => content.extend_from_slice(&buffer[..n]),
                        Err(e) => return Err(ParseMessageError),
                    }
                }

                Ok(MessageType::File {
                    name: "dummy_name".to_string(),
                    content: content,
                })
            }
            s if s.starts_with(".image ") => {
                let path = Path::new(s.strip_prefix(".image ").unwrap_or_default());
                let mut buf_read = BufReader::new(File::open(path).unwrap());
                let mut content: Vec<u8> = Vec::new();
                let mut buffer = [0u8; 1024];

                loop {
                    match buf_read.read(&mut buffer) {
                        Ok(0) => break, // End of file
                        Ok(n) => content.extend_from_slice(&buffer[..n]),
                        Err(e) => return Err(ParseMessageError),
                    }
                }

                Ok(MessageType::Image(content))
            }
            s => Ok(MessageType::Text(s.to_string())),
        }
    }
}

fn listen_and_accept(address: &str) -> Result<()> {
    let listener = TcpListener::bind(address)?;
    println!("Server listening on {}", address);

    let mut clients: HashMap<SocketAddr, TcpStream> = HashMap::new();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => match stream.peer_addr() {
                Ok(addr) => {
                    println!("New connection from {}", addr);

                    clients.insert(addr, stream.try_clone()?);

                    match MessageType::receive_message(stream) {
                        Ok(message) => println!("Received message: {:?}", message),
                        Err(e) => println!("Error receiving message: {}", e),
                    }
                }
                Err(e) => println!("Error getting peer address: {}", e),
            },
            Err(e) => println!("Error accepting connection: {}", e),
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let mode_or_message = env::args().nth(1).unwrap_or_default();

    if mode_or_message == "server" {
        listen_and_accept("127.0.0.1:8080").context("Failed to run server")?;
    } else {
        println!("Client mode: Type messages to send (Ctrl+C to exit)");
        loop {
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;

            let trimmed_input = buf.trim();

            if trimmed_input.starts_with(".quit") {
                println!("Shutting down the client");
                break;
            }

            let message = MessageType::from_str(trimmed_input);

            match message {
                Ok(msg) => msg
                    .send_message("127.0.0.1:8080")
                    .context("Failed to send message to the server")?,
                Err(_) => println!("Can't parse message {}", buf),
            }
        }
    }

    Ok(())
}
