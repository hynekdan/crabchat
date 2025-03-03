use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_cbor::Result as CborResult;

use clap::Parser;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

#[derive(Parser)]
// TODO add some information about the tool
#[command(author, version, about, long_about = None)]
pub struct Cli {
    // using H instead of h to avoid conflict with standard -h for help
    #[arg(short = 'H', long, default_value = "localhost")]
    pub hostname: String,

    #[arg(short, long, default_value_t = 11111)]
    pub port: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum MessageType {
    Text(String),
    Image(Vec<u8>),
    File { name: String, content: Vec<u8> },
}

impl MessageType {
    pub fn serialize(&self) -> CborResult<Vec<u8>> {
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

    pub fn receive_message(stream: &mut TcpStream) -> Result<Self> {
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
    let mut len_bytes = [0u8; 4];

    // Read the message length (as 4-byte value).
    match stream.read_exact(&mut len_bytes) {
        Ok(_) => {}
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
                return Err(anyhow::anyhow!("Failed to read message data: {}", e));
            }
        }
    }

    Ok(buffer)
}

// TODO consider receiving raw message and call serialization from here
pub fn send_serialized_message(stream: &mut TcpStream, serialized: &[u8]) -> Result<()> {
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
pub struct ParseMessageError;

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

// TODO write tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {}
}
