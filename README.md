# CrabChat

A lightweight messaging application built in Rust supporting text, files, and images, with an integrated admin web interface.

## Features
- Real-time text messaging
- File and image transfers
- Memory-safe protocol with size limits
- Built-in protections against oversized messages and invalid inputs
- Asynchronous client and server for high performance
- Persistent storage of users and messages in a database
- Admin web interface for user and message management

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
cargo run -p server -- --admin-port 8081   # Custom admin port
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

## Admin Interface
The CrabChat server includes a web-based admin interface for managing users and messages:

### Accessing the Admin Interface
By default, the admin interface is available at http://localhost:11112 when the server is running.

### Features
- **User Management**: View all users, see who's online, and delete users
- **Message Browsing**: View all messages with filtering by user and message type
- **Message Management**: Delete individual messages
- **User Activity**: See when users first joined and their message counts

### Configuration
Configure the admin interface using command-line options:
```bash
--admin-hostname <HOST>  # Set admin interface hostname (default: localhost)
--admin-port <PORT>      # Set admin interface port (default: 11112)
```

## Project Structure
```
crabchat/
├── Cargo.toml         # Workspace configuration
├── client/            # Client application
├── server/            # Server application
│   ├── src/admin.rs   # Admin web interface
│   └── src/lib.rs     # Server core functionality
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
- **Admin Interface**: Built with Axum web framework
- **Dependencies**: serde, clap, tracing, anyhow, tokio, sqlx, axum

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

### Possible Enhancements

#### User Account Authentication
To improve security, consider adding authentication for user accounts. Possible approaches include:

- **Password Protection**: Require users to register with a username and password.

#### Admin Interface Enhancements
- **Authentication**: Add login requirements for the admin interface
- **Message Content Preview**: For files and images
- **User Statistics**: More detailed analytics on user activity
- **Export Functionality**: Allow exporting message history

## License
MIT License