# CrabChat

A lightweight messaging application built in Rust supporting text, files, and images.

## Features
- Real-time text messaging
- File and image transfers
- Memory-safe protocol with size limits
- Built-in protections against oversized messages and invalid inputs
- Asynchronous client and server for high performance
- Persistent storage of users and messages in a database

## Quick Start

### Installation
```bash
git clone https://github.com/hynekdan/crabchat.git
cd crabchat
cargo build --release
```

### Running
Server:
```bash
cargo run -p server                     # Default (localhost:11111)
cargo run -p server -- -H 0.0.0.0 -p 8080  # Custom host/port
```

Client:
```bash
cargo run -p client                     # Connect to localhost
cargo run -p client -- -H example.com -p 8080  # Remote server
```

### Database Requirements
CrabChat requires a PostgreSQL database for storing users and messages. Ensure you have Docker and Docker Compose installed for easy setup.

1. Start the PostgreSQL database using Docker Compose:
   ```bash
   docker-compose up -d
   ```

   This will start a PostgreSQL container with the required configuration.

2. The server will automatically run database migrations when it starts. No manual migration steps are required.

3. Ensure the `DATABASE_URL` environment variable is set correctly for the server:
   ```bash
   export DATABASE_URL=postgres://username:password@localhost/crabchat
   ```

   Replace `username` and `password` with the credentials configured in your Docker Compose file.

4. Once the database is running, start the server:
   ```bash
   cargo run -p server
   ```

   The server will handle database initialization and migrations automatically.

## Client Commands
- **Text message**: Just type and press Enter
- **Send file**: `.file path/to/file.txt`
- **Send image**: `.image path/to/image.png`
- **Exit**: `.quit`
- **Help**: `.help`

## Project Structure
```
crabchat/
├── Cargo.toml         # Workspace configuration
├── client/            # Client application
├── server/            # Server application
└── utils/             # Shared messaging code
```

## Logging
Control verbosity with the `RUST_LOG` environment variable:
```bash
RUST_LOG=info cargo run -p client    # Normal operation
RUST_LOG=debug cargo run -p server   # Detailed debugging
```

## Technical Details
- **Protocol**: Length-prefixed binary messages with CBOR serialization
- **Message Types**: Text, Image, File
- **Safety**: Built-in protections against oversized messages, invalid inputs, and connection failures
- **Asynchronous Design**: Both client and server are fully async, leveraging `tokio` for high performance
- **Database**: PostgreSQL is used for persistent storage of users and messages
- **Dependencies**: serde, clap, tracing, anyhow, tokio, sqlx

## Development

### Running Tests
Run all tests, including async tests:
```bash
cargo test
```

### Adding Features
To add new features, modify the `MessageType` enum in `utils/src/lib.rs` and implement the corresponding logic in the client and server.

### Example Tests
Some of the implemented tests include:
- **Serialization Tests**: Ensure `MessageType` serialization and deserialization work correctly.
- **File Handling Tests**: Validate file reading, writing, and parsing commands like `.file`.
- **TCP Communication Tests**: Test sending and receiving messages over TCP.

## License
MIT License