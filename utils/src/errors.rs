use std::io;
use thiserror::Error;
use sqlx;

#[derive(Error, Debug)]
pub enum ChatError {
    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Message error: {0}")]
    MessageError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("File error: {context}: {source}")]
    FileError {
        #[source]
        source: io::Error,
        context: String,
    },

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Connection closed by peer")]
    ConnectionClosed,

    #[error("Message too large: {0} bytes (max: 100MB)")]
    MessageTooLarge(usize),

    #[error("Zero-length message received")]
    EmptyMessage,

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<io::Error> for ChatError {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::UnexpectedEof => ChatError::ConnectionClosed,
            io::ErrorKind::ConnectionRefused => {
                ChatError::ConnectionError("Connection refused".to_string())
            }
            io::ErrorKind::ConnectionReset => {
                ChatError::ConnectionError("Connection reset".to_string())
            }
            io::ErrorKind::ConnectionAborted => {
                ChatError::ConnectionError("Connection aborted".to_string())
            }
            io::ErrorKind::NotConnected => ChatError::ConnectionError("Not connected".to_string()),
            io::ErrorKind::TimedOut => {
                ChatError::ConnectionError("Connection timed out".to_string())
            }
            _ => ChatError::FileError {
                source: err,
                context: "I/O operation failed".to_string(),
            },
        }
    }
}

impl From<serde_cbor::Error> for ChatError {
    fn from(err: serde_cbor::Error) -> Self {
        ChatError::SerializationError(err.to_string())
    }
}

impl ChatError {
    pub fn display_error_message(&self) {
        match self {
            ChatError::ConnectionError(msg) => {
                println!("Connection error: {}", msg);
            }
            ChatError::ConnectionClosed => {
                println!("Connection closed by server");
            }
            ChatError::MessageError(msg) => {
                println!("Message error: {}", msg);
            }
            ChatError::FileError { source, context } => {
                println!("File error: {} - {}", context, source);
            }
            ChatError::SerializationError(msg) => {
                println!("Error processing message data: {}", msg);
            }
            ChatError::ProtocolError(msg) => {
                println!("Protocol error: {}", msg);
            }
            ChatError::MessageTooLarge(size) => {
                println!("Message too large ({} bytes). Maximum size is 100MB.", size);
            }
            ChatError::EmptyMessage => {
                println!("Cannot send empty message");
            }
            ChatError::ServerError(msg) => {
                println!("Server error: {}", msg);
            }
            ChatError::InvalidInput(msg) => {
                println!("Invalid input: {}", msg);
            }
            ChatError::AuthError(msg) => {
                println!("Authentication error: {}", msg);
            }
            ChatError::DatabaseError(e) => {
                println!("Database error: {}", e);
            }
            ChatError::Other(e) => {
                println!("Error: {}", e);
            }
        }
    }
}

pub type ChatResult<T> = std::result::Result<T, ChatError>;