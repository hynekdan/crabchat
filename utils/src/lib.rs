use anyhow::{Context, Result as AnyhowResult};
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

pub mod errors;
pub use errors::{ChatError, ChatResult};

#[derive(Parser)]
#[command(
    author = "hynekdan",
    version,
    about = "CrabChat",
    long_about = "A lightweight messaging application built in Rust supporting text, files, and images."
)]
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
    // message type for server->client error reporting
    Error { code: u32, message: String },
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

    pub fn send_message(self, stream: &mut TcpStream) -> AnyhowResult<()> {
        debug!("Preparing to send message");
        let serialized = self
            .serialize()
            .with_context(|| "Failed to serialize message for sending")?;

        debug!("Serialized message size: {} bytes", serialized.len());
        send_serialized_message(stream, &serialized)
            .with_context(|| "Failed to send message to stream")?;

        debug!("Message sent successfully");
        Ok(())
    }

    pub fn receive_message(stream: &mut TcpStream) -> ChatResult<Self> {
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
                    MessageType::Error { code, message } => {
                        debug!(
                            "Deserialized error message: code={}, message={}",
                            code, message
                        )
                    }
                }
                Ok(message)
            }
            Err(e) => {
                error!("Deserialization failed: {}", e);
                Err(ChatError::SerializationError(format!(
                    "Failed to deserialize message: {}",
                    e
                )))
            }
        }
    }

    pub fn handle_received(&self) -> AnyhowResult<()> {
        match self {
            MessageType::Text(text) => {
                info!("Received: {}", text);
                Ok(())
            }
            MessageType::Image(data) => {
                info!("Saving image file ({} bytes)", data.len());
                save_binary_content("images", None, data).with_context(|| {
                    format!("Failed to save received image ({} bytes)", data.len())
                })?;
                Ok(())
            }
            MessageType::File { name, content } => {
                info!("Saving file '{}' ({} bytes)", name, content.len());
                save_binary_content("files", Some(name), content).with_context(|| {
                    format!(
                        "Failed to save received file '{}' ({} bytes)",
                        name,
                        content.len()
                    )
                })?;
                Ok(())
            }
            MessageType::Error { code, message } => {
                error!("Server error (code {}): {}", code, message);
                Ok(()) // Just log the error, no action needed
            }
        }
    }

    pub fn create_error(code: u32, message: &str) -> Self {
        MessageType::Error {
            code,
            message: message.to_string(),
        }
    }
}

fn save_binary_content(dir: &str, filename: Option<&str>, data: &[u8]) -> ChatResult<()> {
    debug!("Creating directory: {}", dir);
    fs::create_dir_all(dir).map_err(|e| ChatError::FileError {
        source: e,
        context: format!("Failed to create directory '{}'", dir),
    })?;

    let path = match filename {
        Some(name) => format!("{}/{}", dir, name),
        None => {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| ChatError::MessageError(format!("Time error: {}", e)))?
                .as_secs();
            format!("{}/{}.png", dir, timestamp)
        }
    };

    info!(
        "Saving {} to path: {}",
        if filename.is_some() { "file" } else { "image" },
        path
    );

    debug!("Creating file: {}", path);
    let mut file = File::create(&path).map_err(|e| ChatError::FileError {
        source: e,
        context: format!("Failed to create file: {}", path),
    })?;

    debug!("Writing {} bytes to file", data.len());
    file.write_all(data).map_err(|e| ChatError::FileError {
        source: e,
        context: format!("Failed to write data to: {}", path),
    })?;

    info!("Successfully saved {} bytes to {}", data.len(), path);

    Ok(())
}

pub fn read_message_from_stream<T>(stream: &mut T) -> ChatResult<Vec<u8>>
where
    T: Read, // Uses Read trait instead of TcpStream to make testing easier while preserving production behavior.
{
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
                return Err(ChatError::ConnectionClosed);
            } else {
                error!("Failed to read message length: {}", e);
                return Err(ChatError::ConnectionError(format!(
                    "Failed to read message length: {}",
                    e
                )));
            }
        }
    }

    let len = u32::from_be_bytes(len_bytes) as usize;
    debug!("Message length from header: {} bytes", len);

    // Sanity check the length to avoid allocating too much memory
    if len == 0 {
        warn!("Received zero-length message");
        return Err(ChatError::EmptyMessage);
    }

    if len > 100_000_000 {
        // 100MB limit
        warn!("Message exceeds size limit: {} bytes (max: 100MB)", len);
        return Err(ChatError::MessageTooLarge(len));
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
                return Err(ChatError::ConnectionClosed);
            }
            Ok(n) => {
                trace!("Read chunk of {} bytes", n);
                bytes_read += n;
            }
            Err(e) => {
                error!("Error reading from stream: {}", e);
                return Err(ChatError::ConnectionError(format!(
                    "Failed to read message data: {}",
                    e
                )));
            }
        }
    }

    debug!("Successfully read complete message: {} bytes", bytes_read);
    Ok(buffer)
}

pub fn send_serialized_message(stream: &mut TcpStream, serialized: &[u8]) -> AnyhowResult<()> {
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

fn read_file_to_vec(path: &Path) -> AnyhowResult<Vec<u8>> {
    debug!("Reading file: {}", path.display());

    let file =
        File::open(path).with_context(|| format!("Failed to open file: {}", path.display()))?;

    let mut buf_read = BufReader::new(file);
    let mut content = Vec::new();

    debug!("Reading file content");
    buf_read
        .read_to_end(&mut content)
        .with_context(|| format!("Failed to read file contents: {}", path.display()))?;

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

#[derive(Debug)]
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

// Error code constants
pub mod error_codes {
    pub const CONNECTION_ERROR: u32 = 1000;
    pub const MESSAGE_ERROR: u32 = 2000;
    pub const PROTOCOL_ERROR: u32 = 3000;
    pub const FILE_ERROR: u32 = 4000;
    pub const SERVER_ERROR: u32 = 5000;
    pub const CLIENT_ERROR: u32 = 6000;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::thread;
    use tempfile::tempdir;

    // Helper function to create a temporary file with content
    fn create_temp_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let file_path = dir.join(name);
        let mut file = File::create(&file_path).unwrap();
        file.write_all(content).unwrap();
        file_path
    }

    #[test]
    fn test_message_type_serialization_text() {
        let message = MessageType::Text("Hello, world!".to_string());

        let serialized = message.serialize().unwrap();
        let deserialized = MessageType::deserialize(&serialized).unwrap();

        match deserialized {
            MessageType::Text(text) => assert_eq!(text, "Hello, world!"),
            _ => panic!("Deserialized to wrong message type"),
        }
    }

    #[test]
    fn test_message_type_serialization_image() {
        let image_data = vec![0, 1, 2, 3, 4, 5]; // Mock image data
        let message = MessageType::Image(image_data.clone());

        let serialized = message.serialize().unwrap();
        let deserialized = MessageType::deserialize(&serialized).unwrap();

        match deserialized {
            MessageType::Image(data) => assert_eq!(data, image_data),
            _ => panic!("Deserialized to wrong message type"),
        }
    }

    #[test]
    fn test_message_type_serialization_file() {
        let file_name = "test.txt".to_string();
        let file_content = vec![10, 20, 30, 40, 50]; // Mock file content
        let message = MessageType::File {
            name: file_name.clone(),
            content: file_content.clone(),
        };

        let serialized = message.serialize().unwrap();
        let deserialized = MessageType::deserialize(&serialized).unwrap();

        match deserialized {
            MessageType::File { name, content } => {
                assert_eq!(name, file_name);
                assert_eq!(content, file_content);
            }
            _ => panic!("Deserialized to wrong message type"),
        }
    }

    #[test]
    fn test_save_binary_content() {
        let temp_dir = tempdir().unwrap();
        let test_dir = temp_dir.path().join("test_dir");

        let data = vec![1, 2, 3, 4, 5];
        let filename = "test_file.bin";

        save_binary_content(test_dir.to_str().unwrap(), Some(filename), &data).unwrap();

        let mut saved_path = test_dir.clone();
        saved_path.push(filename);
        assert!(saved_path.exists());

        let saved_content = fs::read(&saved_path).unwrap();
        assert_eq!(saved_content, data);
    }

    #[test]
    fn test_read_file_to_vec() {
        let temp_dir = tempdir().unwrap();
        let test_content = b"This is a test file";
        let file_path = create_temp_file(temp_dir.path(), "test_read.txt", test_content);

        let content = read_file_to_vec(&file_path).unwrap();

        assert_eq!(content, test_content);
    }

    #[test]
    fn test_get_filename_as_string() {
        let path = Path::new("/path/to/some/file.txt");
        let filename = get_filename_as_string(path);
        assert_eq!(filename, "file.txt");
    }

    #[test]
    fn test_parse_text_message() {
        let input = "This is a test message";
        let message = MessageType::from_str(input).unwrap();

        match message {
            MessageType::Text(text) => assert_eq!(text, input),
            _ => panic!("Parsed to wrong message type"),
        }
    }

    #[test]
    fn test_parse_file_message() {
        let temp_dir = tempdir().unwrap();
        let test_content = b"Test file content";
        let file_path = create_temp_file(temp_dir.path(), "test_parse.txt", test_content);

        let command = format!(".file {}", file_path.display());

        let message = MessageType::from_str(&command).unwrap();

        match message {
            MessageType::File { name, content } => {
                assert_eq!(name, "test_parse.txt");
                assert_eq!(content, test_content);
            }
            _ => panic!("Parsed to wrong message type"),
        }
    }

    #[test]
    fn test_parse_image_message() {
        let temp_dir = tempdir().unwrap();
        let test_content = b"Fake image data";
        let file_path = create_temp_file(temp_dir.path(), "test_image.png", test_content);

        let command = format!(".image {}", file_path.display());

        let message = MessageType::from_str(&command).unwrap();

        match message {
            MessageType::Image(content) => {
                assert_eq!(content, test_content);
            }
            _ => panic!("Parsed to wrong message type"),
        }
    }

    #[test]
    fn test_parse_invalid_file() {
        let command = ".file /path/to/nonexistent/file.txt";
        let result = MessageType::from_str(command);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_write_tcp() {
        // Start a TCP server
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server_thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();

            let message = MessageType::receive_message(&mut stream).unwrap();

            match message {
                MessageType::Text(text) => assert_eq!(text, "Test TCP message"),
                _ => panic!("Received wrong message type"),
            }

            let response = MessageType::Text("Response from server".to_string());
            response.send_message(&mut stream).unwrap();
        });

        // Connect to the server
        let mut client = TcpStream::connect(addr).unwrap();

        // Send a message
        let message = MessageType::Text("Test TCP message".to_string());
        message.send_message(&mut client).unwrap();

        // Receive the response
        let response = MessageType::receive_message(&mut client).unwrap();

        match response {
            MessageType::Text(text) => assert_eq!(text, "Response from server"),
            _ => panic!("Received wrong message type"),
        }

        server_thread.join().unwrap();
    }

    #[test]
    fn test_read_message_empty_stream() {
        // Test reading from an empty stream
        let data: Vec<u8> = vec![];
        let mut cursor = Cursor::new(data);

        let result = read_message_from_stream(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_message_invalid_length() {
        // Create a stream with an invalid length prefix (too large)
        let mut data = Vec::new();
        let len: u32 = 200_000_000; // 200MB (exceeds limit)
        data.extend_from_slice(&len.to_be_bytes());

        let mut cursor = Cursor::new(data);

        let result = read_message_from_stream(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_message_zero_length() {
        // Create a stream with a zero length prefix
        let mut data = Vec::new();
        let len: u32 = 0;
        data.extend_from_slice(&len.to_be_bytes());

        let mut cursor = Cursor::new(data);

        let result = read_message_from_stream(&mut cursor);
        assert!(result.is_err());
    }
}
