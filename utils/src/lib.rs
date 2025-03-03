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
use tracing::{debug, error, info, trace, warn};

#[derive(Parser)]
#[command(author="hynekdan", version, about="CrabChat", long_about = "A lightweight messaging application built in Rust supporting text, files, and images.")]
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
        let result = serde_cbor::to_vec(&self);
        if let Err(ref e) = result {
            error!("Failed to serialize message: {}", e);
        } else {
            debug!("Successfully serialized message");
        }
        result
    }

    fn deserialize(input: &[u8]) -> CborResult<Self> {
        let result = serde_cbor::from_slice(input);
        if let Err(ref e) = result {
            error!("Failed to deserialize data: {}", e);
        } else {
            debug!("Successfully deserialized message");
        }
        result
    }

    pub fn send_message(self, stream: &mut TcpStream) -> Result<()> {
        debug!("Preparing to send message");
        let serialized = self.serialize()?;
        debug!("Serialized message size: {} bytes", serialized.len());
        send_serialized_message(stream, &serialized)?;
        debug!("Message sent successfully");
        Ok(())
    }

    pub fn receive_message(stream: &mut TcpStream) -> Result<Self> {
        debug!("Receiving message from stream");
        let buffer = read_message_from_stream(stream)?;
        debug!("Received raw data: {} bytes", buffer.len());

        match Self::deserialize(&buffer) {
            Ok(message) => {
                match &message {
                    MessageType::Text(text) => debug!("Deserialized text message: {}", text),
                    MessageType::Image(data) => debug!("Deserialized image: {} bytes", data.len()),
                    MessageType::File { name, content } => {
                        debug!("Deserialized file '{}': {} bytes", name, content.len())
                    }
                }
                Ok(message)
            }
            Err(e) => {
                error!("Deserialization failed: {}", e);
                Err(anyhow::anyhow!("Failed to deserialize message: {}", e))
            }
        }
    }

    pub fn handle_received(&self) -> Result<()> {
        match self {
            MessageType::Text(text) => {
                info!("Received: {}", text);
            }
            MessageType::Image(data) => {
                info!("Saving image file ({} bytes)", data.len());
                save_binary_content("images", None, data)?;
            }
            MessageType::File { name, content } => {
                info!("Saving file '{}' ({} bytes)", name, content.len());
                save_binary_content("files", Some(name), content)?;
            }
        }
        Ok(())
    }
}

fn save_binary_content(dir: &str, filename: Option<&str>, data: &[u8]) -> Result<()> {
    debug!("Creating directory: {}", dir);
    fs::create_dir_all(dir)?;

    let path = match filename {
        Some(name) => format!("{}/{}", dir, name),
        None => {
            let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            format!("{}/{}.png", dir, timestamp)
        }
    };

    info!(
        "Saving {} to path: {}",
        if filename.is_some() { "file" } else { "image" },
        path
    );

    debug!("Creating file: {}", path);
    let mut file =
        File::create(&path).with_context(|| format!("Failed to create file: {}", path))?;

    debug!("Writing {} bytes to file", data.len());
    file.write_all(data)
        .with_context(|| format!("Failed to write data to: {}", path))?;

    info!("Successfully saved {} bytes to {}", data.len(), path);

    Ok(())
}

fn read_message_from_stream(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];

    debug!("Reading message length prefix (4 bytes)");
    // Read the message length (as 4-byte value).
    match stream.read_exact(&mut len_bytes) {
        Ok(_) => {
            debug!("Successfully read length prefix");
        }
        Err(e) => {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                error!("Connection closed by peer while reading message length");
                return Err(anyhow::anyhow!("Connection closed by peer"));
            } else {
                error!("Failed to read message length: {}", e);
                return Err(anyhow::anyhow!("Failed to read message length: {}", e));
            }
        }
    }

    let len = u32::from_be_bytes(len_bytes) as usize;
    debug!("Message length from header: {} bytes", len);

    // Sanity check the length to avoid allocating too much memory
    if len == 0 {
        warn!("Received zero-length message");
        return Err(anyhow::anyhow!("Zero-length message received"));
    }

    if len > 100_000_000 {
        // 100MB limit
        warn!("Message exceeds size limit: {} bytes (max: 100MB)", len);
        return Err(anyhow::anyhow!("Message too large: {} bytes", len));
    }

    debug!("Allocating buffer for {} bytes", len);
    let mut buffer = vec![0u8; len];

    // Read the full message with better error handling
    let mut bytes_read = 0;
    while bytes_read < len {
        trace!("Reading data chunk at offset {}/{}", bytes_read, len);
        match stream.read(&mut buffer[bytes_read..]) {
            Ok(0) => {
                error!(
                    "Connection closed after reading {} of {} bytes",
                    bytes_read, len
                );
                return Err(anyhow::anyhow!(
                    "Connection closed before reading full message"
                ));
            }
            Ok(n) => {
                trace!("Read chunk of {} bytes", n);
                bytes_read += n;
            }
            Err(e) => {
                error!("Error reading from stream: {}", e);
                return Err(anyhow::anyhow!("Failed to read message data: {}", e));
            }
        }
    }

    debug!("Successfully read complete message: {} bytes", bytes_read);
    Ok(buffer)
}

pub fn send_serialized_message(stream: &mut TcpStream, serialized: &[u8]) -> Result<()> {
    // Send the length of the serialized message (as 4-byte value).
    let len = serialized.len() as u32;
    debug!("Sending message length: {} bytes", len);

    stream
        .write_all(&len.to_be_bytes())
        .with_context(|| "Failed to send message length")?;

    // Send the serialized message.
    debug!("Sending message payload");
    stream
        .write_all(serialized)
        .with_context(|| "Failed to send message payload")?;

    debug!("Flushing stream");
    stream.flush().with_context(|| "Failed to flush stream")?;

    debug!("Successfully sent message of {} bytes", len);
    Ok(())
}

fn read_file_to_vec(path: &Path) -> Result<Vec<u8>> {
    debug!("Reading file: {}", path.display());

    let file =
        File::open(path).with_context(|| format!("Failed to open file: {}", path.display()))?;

    let mut buf_read = BufReader::new(file);
    let mut content = Vec::new();

    debug!("Reading file content");
    buf_read
        .read_to_end(&mut content)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    debug!(
        "Successfully read {} bytes from {}",
        content.len(),
        path.display()
    );
    Ok(content)
}

fn get_filename_as_string(path: &Path) -> String {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or_default()
        .to_string();

    debug!(
        "Extracted filename '{}' from path: {}",
        filename,
        path.display()
    );
    filename
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseMessageError;

impl FromStr for MessageType {
    type Err = ParseMessageError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            s if s.starts_with(".file ") => {
                let path_str = s.strip_prefix(".file ").unwrap_or_default();
                let path = Path::new(path_str);
                debug!("Parsing file command for path: {}", path.display());

                match read_file_to_vec(path) {
                    Ok(content) => {
                        let name = get_filename_as_string(path);
                        debug!(
                            "Created file message with name: '{}', size: {} bytes",
                            name,
                            content.len()
                        );
                        Ok(MessageType::File { name, content })
                    }
                    Err(e) => {
                        error!("Failed to read file {}: {}", path.display(), e);
                        Err(ParseMessageError)
                    }
                }
            }
            s if s.starts_with(".image ") => {
                let path_str = s.strip_prefix(".image ").unwrap_or_default();
                let path = Path::new(path_str);
                debug!("Parsing image command for path: {}", path.display());

                match read_file_to_vec(path) {
                    Ok(content) => {
                        debug!("Created image message with size: {} bytes", content.len());
                        Ok(MessageType::Image(content))
                    }
                    Err(e) => {
                        error!("Failed to read image {}: {}", path.display(), e);
                        Err(ParseMessageError)
                    }
                }
            }
            s => {
                debug!("Created text message: {}", s);
                Ok(MessageType::Text(s.to_string()))
            }
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
