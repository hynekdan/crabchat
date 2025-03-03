# CrabChat

A lightweight messaging application built in Rust supporting text, files, and images.

## Features
- Real-time text messaging
- File and image transfers 
- Memory-safe protocol with size limits

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

## Client Commands
- Text message: Just type and press Enter
- Send file: `.file path/to/file.txt`
- Send image: `.image path/to/image.png`
- Exit: `.quit`

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
- **Safety**: Built-in protections against oversized messages and connection failures
- **Dependencies**: serde, clap, tracing, anyhow

## Development

Run tests (WIP):
```bash
cargo test
```

Add features by modifying `MessageType` enum in `utils/src/lib.rs`.

## License
MIT License