//! Utility library for the CrabChat application.
//!
//! This module provides shared functionality for both the client and server, including:
//! - Message serialization and deserialization
//! - File and image handling
//! - Network protocol helpers
//! - Command-line argument parsing
//!
//! # Examples
//!
//! ## Serializing and Deserializing Messages
//! ```rust
//! use utils::MessageType;
//!
//! let message = MessageType::Text("Hello, CrabChat!".to_string());
//! let serialized = message.serialize().unwrap();
//! let deserialized = MessageType::deserialize(&serialized).unwrap();
//!
//! match deserialized {
//!     MessageType::Text(text) => assert_eq!(text, "Hello, CrabChat!"),
//!     _ => panic!("Unexpected message type"),
//! }
//! ```

use anyhow::{Context, Result as AnyhowResult};
use serde::{Deserialize, Serialize};
use serde_cbor::Result as CborResult;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs::{self, File};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tracing::{debug, error, info, trace, warn};

pub mod errors;
pub use errors::{ChatError, ChatResult};

use clap::Parser;

/// Command-line arguments for the CrabChat application.
///
/// This struct is used to parse and store command-line arguments for both the client and server.
#[derive(Parser, Debug)]
#[command(
    author = "hynekdan",
    version,
    about = "CrabChat",
    long_about = "A lightweight messaging application built in Rust supporting text, files, and images."
)]
pub struct Cli {
    // using H instead of h to avoid conflict with standard -h for help
    /// Hostname or IP address to connect to (default: localhost).
    #[arg(short = 'H', long, default_value = "localhost")]
    pub hostname: String,

    /// Port number to connect to (default: 11111).
    #[arg(short, long, default_value_t = 11111)]
    pub port: u16,

    /// Username for the client (for the client, not used by the server).
    #[arg(short, long)]
    pub username: Option<String>,

    /// Hostname or IP address for the admin interface (default: localhost).
    #[arg(long, default_value = "localhost")]
    pub admin_hostname: String,

    /// Port number for the admin interface (default: 11112).
    #[arg(long, default_value_t = 11112)]
    pub admin_port: u16,

    /// Hostname or IP address for the metrics interface (default: localhost).
    #[arg(long, default_value = "localhost")]
    pub metrics_hostname: String,

    /// Port number for the metrics interface (default: 11113).
    #[arg(long, default_value_t = 11113)]
    pub metrics_port: u16,
}

/// Represents the types of messages exchanged between the client and server.
///
/// This enum includes variants for text messages, file transfers, image transfers,
/// and error messages.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum MessageType {
    /// Login message containing the username.
    Login(String),

    /// Acknowledgment of a successful login.
    LoginOk,

    /// A text message.
    Text(String),

    /// An image message containing binary data.
    Image(Vec<u8>),

    /// A file message containing the file name and binary content.
    File { name: String, content: Vec<u8> },

    /// An error message containing a description of the error.
    Error(String),

    /// A local command (used only on the client side).
    #[serde(skip)]
    LocalCommand(LocalCommandType),
}

/// Represents local commands that can be executed by the client.
///
/// These commands are not sent over the network.
#[derive(Debug, Clone)]
pub enum LocalCommandType {
    /// Command to send a file.
    SendFile(PathBuf),

    /// Command to send an image.
    SendImage(PathBuf),

    /// Command to quit the application.
    Quit,

    /// Command to display help information.
    Help,
}

impl MessageType {
    /// Serializes the message into a CBOR-encoded byte vector.
    ///
    /// # Examples
    /// ```rust
    /// use utils::MessageType;
    ///
    /// let message = MessageType::Text("Hello, CrabChat!".to_string());
    /// let serialized = message.serialize().unwrap();
    /// assert!(!serialized.is_empty());
    /// ```
    pub fn serialize(&self) -> CborResult<Vec<u8>> {
        let result = serde_cbor::to_vec(&self);
        if let Err(ref e) = result {
            error!("Failed to serialize message: {}", e);
        } else {
            debug!("Successfully serialized message");
        }
        result
    }

    /// Deserializes a CBOR-encoded byte vector into a `MessageType`.
    pub fn deserialize(input: &[u8]) -> CborResult<Self> {
        let result = serde_cbor::from_slice(input);
        if let Err(ref e) = result {
            error!("Failed to deserialize data: {}", e);
        } else {
            debug!("Successfully deserialized message");
        }
        result
    }

    // --- Async Network Operations ---

    /// Asynchronously sends a message prefixed with its length.
    ///
    /// # Arguments
    /// - `stream`: The async stream to send the message to.
    ///
    /// # Errors
    /// Returns an error if serialization or sending fails.
    pub async fn send<W>(&self, stream: &mut W) -> ChatResult<()>
    where
        W: AsyncWrite + Unpin + ?Sized,
    {
        let serialized = self
            .serialize()
            .map_err(|e| ChatError::SerializationError(format!("Serialize failed: {}", e)))?;

        send_serialized_message(stream, &serialized).await
    }

    /// Asynchronously receives a length-prefixed message.
    ///
    /// # Arguments
    /// - `stream`: The async stream to receive the message from.
    ///
    /// # Errors
    /// Returns an error if deserialization or receiving fails.
    pub async fn receive<R>(stream: &mut R) -> ChatResult<Self>
    where
        R: AsyncRead + Unpin + ?Sized,
    {
        let buffer = read_message_from_stream(stream).await?;
        debug!("Received raw data: {} bytes", buffer.len());

        match Self::deserialize(&buffer) {
            Ok(message) => {
                trace!("Deserialized message: {:?}", message);
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

    // --- Async Client-Side Handling ---
    /// Handles received message on the client side (async for file I/O)
    pub async fn handle_received_client(&self, save_dir: &Path) -> AnyhowResult<()> {
        match self {
            MessageType::LoginOk => {
                info!("Successfully logged in!");
                Ok(())
            }
            MessageType::Text(text) => {
                info!("{}", text); // Assume text includes sender info from server
                Ok(())
            }
            MessageType::Image(data) => {
                let img_dir = save_dir.join("images");
                info!(
                    "Saving image file ({} bytes) to {}",
                    data.len(),
                    img_dir.display()
                );
                save_binary_content(&img_dir, None, data).await?;
                info!("Image saved.");
                Ok(())
            }
            MessageType::File { name, content } => {
                let file_dir = save_dir.join("files");
                info!(
                    "Saving file '{}' ({} bytes) to {}",
                    name,
                    content.len(),
                    file_dir.display()
                );
                save_binary_content(&file_dir, Some(name), content).await?;
                info!("File '{}' saved.", name);
                Ok(())
            }
            MessageType::Error(msg) => {
                error!("Received error: {}", msg);
                Ok(())
            }
            MessageType::Login(_) => {
                warn!("Client should not receive Login messages");
                Ok(())
            }
            MessageType::LocalCommand(_) => {
                // Should not be received over network
                unreachable!("LocalCommand should not be received");
            }
        }
    }
}

// --- Async File I/O ---

/// Asynchronously saves binary content to a specified directory.
///
/// # Arguments
/// - `dir`: The directory to save the file in.
/// - `filename`: The name of the file (optional).
/// - `data`: The binary data to save.
///
/// # Errors
/// Returns an error if the directory or file cannot be created.
///
/// # Examples
/// ```rust
/// use utils::save_binary_content;
/// use std::path::Path;
/// use tempfile::tempdir;
///
/// #[tokio::main]
/// async fn main() {
///     let data = b"Hello, world!";
///     let temp_dir = tempdir().unwrap();
///     save_binary_content(temp_dir.path(), Some("test.txt"), data).await.unwrap();
/// }
/// ```
pub async fn save_binary_content(
    dir: &Path,
    filename: Option<&str>,
    data: &[u8],
) -> ChatResult<()> {
    debug!("Ensuring directory exists: {}", dir.display());
    fs::create_dir_all(dir)
        .await
        .map_err(|e| ChatError::FileError {
            source: e,
            context: format!("Failed to create directory '{}'", dir.display()),
        })?;

    let file_path = match filename {
        Some(name) => dir.join(name),
        None => {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| ChatError::MessageError(format!("Time error: {}", e)))?
                .as_millis(); // Use different format if uniqueness is a problem
            dir.join(format!("{}.png", timestamp))
        }
    };

    debug!("Creating file: {}", file_path.display());
    let mut file = File::create(&file_path)
        .await
        .map_err(|e| ChatError::FileError {
            source: e,
            context: format!("Failed to create file: {}", file_path.display()),
        })?;

    debug!("Writing {} bytes to file", data.len());
    file.write_all(data)
        .await
        .map_err(|e| ChatError::FileError {
            source: e,
            context: format!("Failed to write data to: {}", file_path.display()),
        })?;

    file.flush().await.map_err(|e| ChatError::FileError {
        source: e,
        context: format!("Failed to flush data to: {}", file_path.display()),
    })?;

    debug!(
        "Successfully saved {} bytes to {}",
        data.len(),
        file_path.display()
    );
    Ok(())
}

/// Asynchronously reads an entire file into a byte vector.
///
/// # Arguments
/// - `path`: The path to the file to read.
///
/// # Errors
/// Returns an error if the file cannot be opened or read.
pub async fn read_file_to_vec(path: &Path) -> AnyhowResult<Vec<u8>> {
    debug!("Reading file asynchronously: {}", path.display());

    let file = File::open(path)
        .await
        .with_context(|| format!("Failed to open file: {}", path.display()))?;

    let mut buf_read = BufReader::new(file);
    let mut content = Vec::new();

    buf_read
        .read_to_end(&mut content)
        .await
        .with_context(|| format!("Failed to read file contents: {}", path.display()))?;

    debug!(
        "Successfully read {} bytes from {}",
        content.len(),
        path.display()
    );
    Ok(content)
}

/// Extracts filename, returning an empty string on failure (unchanged logic).
pub fn get_filename_as_string(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or_default()
        .to_string()
}

// --- Async Network Protocol Helpers ---

/// Reads a length-prefixed message from an async stream. Length prefixing uses 4 bytes in big-endian format.
///
/// # Arguments
/// - `stream`: The async stream to read the message from.
///
/// # Errors
/// Returns an error if the message length is invalid, the connection is closed, or the message cannot be read.
///
/// # Examples
/// ```rust
/// use utils::{read_message_from_stream, MessageType};
/// use std::io::Cursor;
///
/// #[tokio::main]
/// async fn main() {
///     let message = MessageType::Text("Hello, CrabChat!".to_string());
///     let serialized = message.serialize().unwrap();
///
///     let mut data = Vec::new();
///     let len = serialized.len() as u32;
///     data.extend_from_slice(&len.to_be_bytes());
///     data.extend_from_slice(&serialized);
///
///     let mut cursor = Cursor::new(data);
///     let received = read_message_from_stream(&mut cursor).await.unwrap();
///     assert_eq!(received, serialized);
/// }
/// ```
pub async fn read_message_from_stream<R>(stream: &mut R) -> ChatResult<Vec<u8>>
where
    R: AsyncRead + Unpin + ?Sized,
{
    let mut len_bytes = [0u8; 4];
    debug!("Reading message length prefix (4 bytes)");

    match stream.read_exact(&mut len_bytes).await {
        Ok(_) => {
            trace!("Successfully read length prefix bytes: {:?}", len_bytes);
        }
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            warn!("Connection closed by peer while reading message length");
            return Err(ChatError::ConnectionClosed);
        }
        Err(e) => {
            error!("Failed to read message length: {}", e);
            return Err(ChatError::ConnectionError(format!(
                "Failed to read message length: {}",
                e
            )));
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
    match stream.read_exact(&mut buffer).await {
        Ok(_) => {
            debug!("Successfully read complete message: {} bytes", buffer.len());
            Ok(buffer)
        }
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            error!(
                "Connection closed unexpectedly after reading header (expected {} bytes)",
                len
            );
            Err(ChatError::ConnectionClosed)
        }
        Err(e) => {
            error!("Failed to read message body ({} bytes): {}", len, e);
            Err(ChatError::ConnectionError(format!(
                "Failed to read message body: {}",
                e
            )))
        }
    }
}

/// Sends a length-prefixed message to an async stream. Length prefixing uses 4 bytes in big-endian format.
///
/// # Arguments
/// - `stream`: The async stream to send the message to.
/// - `serialized`: The serialized message to send.
///
/// # Errors
/// Returns an error if the message cannot be sent or the connection is closed.
pub async fn send_serialized_message<W>(stream: &mut W, serialized: &[u8]) -> ChatResult<()>
where
    W: AsyncWrite + Unpin + ?Sized,
{
    let len = serialized.len() as u32;
    debug!("Sending message length: {} bytes", len);
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| ChatError::ConnectionError(format!("Failed to send message length: {}", e)))?;

    debug!("Sending message payload ({} bytes)", len);
    stream.write_all(serialized).await.map_err(|e| {
        ChatError::ConnectionError(format!("Failed to send message payload: {}", e))
    })?;

    // Flushing might be important depending on underlying buffering
    stream
        .flush()
        .await
        .map_err(|e| ChatError::ConnectionError(format!("Failed to flush stream: {}", e)))?;

    debug!("Successfully sent message of {} bytes", len);
    Ok(())
}

// --- Input Parsing (Moved out of FromStr) ---

/// Parses user input into a LocalCommandType or a simple text string.
pub fn parse_input(input: &str) -> Result<MessageType, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Input is empty".to_string());
    }

    if let Some(path_str) = trimmed.strip_prefix(".file ") {
        let path = PathBuf::from(path_str.trim());
        if path.is_file() {
            // Basic check
            Ok(MessageType::LocalCommand(LocalCommandType::SendFile(path)))
        } else {
            Err(format!(
                "File not found or is not a file: {}",
                path.display()
            ))
        }
    } else if let Some(path_str) = trimmed.strip_prefix(".image ") {
        let path = PathBuf::from(path_str.trim());
        if path.is_file() {
            // Basic check
            Ok(MessageType::LocalCommand(LocalCommandType::SendImage(path)))
        } else {
            Err(format!(
                "Image not found or is not a file: {}",
                path.display()
            ))
        }
    } else if trimmed == ".quit" {
        Ok(MessageType::LocalCommand(LocalCommandType::Quit))
    } else if trimmed == ".help" {
        Ok(MessageType::LocalCommand(LocalCommandType::Help))
    } else if trimmed.starts_with('.') {
        Err(format!("Unknown command: {}", trimmed))
    } else {
        Ok(MessageType::Text(trimmed.to_string()))
    }
}

pub mod error_codes {
    pub const CONNECTION_ERROR: u32 = 1000;
    pub const MESSAGE_ERROR: u32 = 2000;
    pub const PROTOCOL_ERROR: u32 = 3000;
    pub const FILE_ERROR: u32 = 4000;
    pub const SERVER_ERROR: u32 = 5000;
    pub const CLIENT_ERROR: u32 = 6000;
    pub const AUTH_ERROR: u32 = 7000;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;
    use tempfile::tempdir;
    use tokio::fs;
    use tokio::net::{TcpListener, TcpStream};

    // Helper function to create a temporary file with content
    fn create_temp_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let file_path = dir.join(name);
        let mut file = std::fs::File::create(&file_path).unwrap();
        use std::io::Write;
        file.write_all(content).unwrap();
        file_path
    }

    #[tokio::test]
    async fn test_message_type_serialization_text() {
        let message = MessageType::Text("Hello, world!".to_string());

        let serialized = message.serialize().unwrap();
        let deserialized = MessageType::deserialize(&serialized).unwrap();

        match deserialized {
            MessageType::Text(text) => assert_eq!(text, "Hello, world!"),
            _ => panic!("Deserialized to wrong message type"),
        }
    }

    #[tokio::test]
    async fn test_message_type_serialization_image() {
        let image_data = vec![0, 1, 2, 3, 4, 5]; // Mock image data
        let message = MessageType::Image(image_data.clone());

        let serialized = message.serialize().unwrap();
        let deserialized = MessageType::deserialize(&serialized).unwrap();

        match deserialized {
            MessageType::Image(data) => assert_eq!(data, image_data),
            _ => panic!("Deserialized to wrong message type"),
        }
    }

    #[tokio::test]
    async fn test_message_type_serialization_file() {
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

    #[tokio::test]
    async fn test_save_binary_content() {
        let temp_dir = tempdir().unwrap();
        let test_dir = temp_dir.path().join("test_dir");

        let data = vec![1, 2, 3, 4, 5];
        let filename = "test_file.bin";

        save_binary_content(&test_dir, Some(filename), &data)
            .await
            .unwrap();

        let saved_path = test_dir.join(filename);
        assert!(saved_path.exists());

        let saved_content = fs::read(&saved_path).await.unwrap();
        assert_eq!(saved_content, data);
    }

    #[tokio::test]
    async fn test_read_file_to_vec() {
        let temp_dir = tempdir().unwrap();
        let test_content = b"This is a test file";
        let file_path = create_temp_file(temp_dir.path(), "test_read.txt", test_content);

        let content = read_file_to_vec(&file_path).await.unwrap();

        assert_eq!(content, test_content);
    }

    #[test]
    fn test_get_filename_as_string() {
        let path = Path::new("/path/to/some/file.txt");
        let filename = get_filename_as_string(path);
        assert_eq!(filename, "file.txt");
    }

    #[tokio::test]
    async fn test_parse_text_message() {
        let input = "This is a test message";
        let message = parse_input(input).unwrap();

        match message {
            MessageType::Text(text) => assert_eq!(text, input),
            _ => panic!("Parsed to wrong message type"),
        }
    }

    #[tokio::test]
    async fn test_parse_file_message() {
        let temp_dir = tempdir().unwrap();
        let test_content = b"Test file content";
        let file_path = create_temp_file(temp_dir.path(), "test_parse.txt", test_content);

        let command = format!(".file {}", file_path.display());

        let message = parse_input(&command).unwrap();

        match message {
            MessageType::LocalCommand(LocalCommandType::SendFile(path)) => {
                assert_eq!(path.file_name().unwrap(), "test_parse.txt");
            }
            _ => panic!("Parsed to wrong message type"),
        }
    }

    #[tokio::test]
    async fn test_parse_image_message() {
        let temp_dir = tempdir().unwrap();
        let test_content = b"Fake image data";
        let file_path = create_temp_file(temp_dir.path(), "test_image.png", test_content);

        let command = format!(".image {}", file_path.display());

        let message = parse_input(&command).unwrap();

        match message {
            MessageType::LocalCommand(LocalCommandType::SendImage(path)) => {
                assert_eq!(path.file_name().unwrap(), "test_image.png");
            }
            _ => panic!("Parsed to wrong message type"),
        }
    }

    #[tokio::test]
    async fn test_parse_invalid_file() {
        let command = ".file /path/to/nonexistent/file.txt";
        let result = parse_input(command);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_write_tcp() {
        // Start a TCP server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server_thread = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            let message = MessageType::receive(&mut stream).await.unwrap();

            match message {
                MessageType::Text(text) => assert_eq!(text, "Test TCP message"),
                _ => panic!("Received wrong message type"),
            }

            let response = MessageType::Text("Response from server".to_string());
            response.send(&mut stream).await.unwrap();
        });

        // Connect to the server
        let mut client = TcpStream::connect(addr).await.unwrap();

        // Send a message
        let message = MessageType::Text("Test TCP message".to_string());
        message.send(&mut client).await.unwrap();

        // Receive the response
        let response = MessageType::receive(&mut client).await.unwrap();

        match response {
            MessageType::Text(text) => assert_eq!(text, "Response from server"),
            _ => panic!("Received wrong message type"),
        }

        server_thread.await.unwrap();
    }

    #[tokio::test]
    async fn test_read_message_empty_stream() {
        // Test reading from an empty stream
        let data: Vec<u8> = vec![];
        let mut cursor = Cursor::new(data);

        let result = read_message_from_stream(&mut cursor).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_message_invalid_length() {
        // Create a stream with an invalid length prefix (too large)
        let mut data = Vec::new();
        let len: u32 = 200_000_000; // 200MB (exceeds limit)
        data.extend_from_slice(&len.to_be_bytes());

        let mut cursor = Cursor::new(data);

        let result = read_message_from_stream(&mut cursor).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_message_zero_length() {
        // Create a stream with a zero length prefix
        let mut data = Vec::new();
        let len: u32 = 0;
        data.extend_from_slice(&len.to_be_bytes());

        let mut cursor = Cursor::new(data);

        let result = read_message_from_stream(&mut cursor).await;
        assert!(result.is_err());
    }
}
