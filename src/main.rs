use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Result as JsonResult;

use std::collections::HashMap;
use std::env;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

#[derive(Serialize, Deserialize, Debug)]
enum MessageType {
    Text(String),
    Image(Vec<u8>),
    File { name: String, content: Vec<u8> },
}

impl MessageType {
    fn serialize(&self) -> JsonResult<String> {
        let serialized = serde_json::to_string(&self)?;

        Ok(serialized)
    }

    fn deserialize<S: AsRef<str>>(input: &S) -> JsonResult<Self> {
        let string = input.as_ref();

        serde_json::from_str(string)
    }

    pub fn send_message(self, address: &str) -> Result<()> {
        let serialized = self.serialize()?;
        let mut stream = TcpStream::connect(address)?;

        // Send the length of the serialized message (as 4-byte value).
        let len = serialized.as_bytes().len() as u32;
        stream.write(&len.to_be_bytes())?;

        // Send the serialized message.
        stream.write_all(serialized.as_bytes())?;

        Ok(())
    }

    pub fn receive_message(mut stream: TcpStream) -> Result<Self> {
        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes)?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        let mut buffer = vec![0u8; len];
        stream.read_exact(&mut buffer)?;

        let string = String::from_utf8(buffer)?;

        // JsonResult -> anyhow::Result
        Ok(Self::deserialize(&string)?)
    }
}

fn listen_and_accept(address: &str) -> Result<()> {
    let listener = TcpListener::bind(address)?;

    let mut clients: HashMap<SocketAddr, TcpStream> = HashMap::new();

    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let Ok(addr) = stream.peer_addr() else {
            continue;
        };

        clients.insert(addr.clone(), stream);

        let client = if let Some(client_ref) = clients.get(&addr) {
            let clone = client_ref.try_clone();
            match clone {
                Err(_) => {
                    println!("Stream of {addr} is no longer valid");
                    clients.remove(&addr);
                    continue;
                }
                Ok(cloned) => cloned,
            }
        } else {
            unreachable!("The unlikely to happen has happened")
        };

        let message = MessageType::receive_message(client);
        // Here, you can further process this message as per your requirements
        println!("{:?}", message);
    }

    Ok(())
}

fn main() -> Result<()> {
    let mode_or_message = env::args().nth(1).unwrap_or_default();

    if mode_or_message == "server" {
        listen_and_accept("127.0.0.1:8080").context("Failed to run server")?;
    } else {
        loop {
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;

            let new_message = MessageType::Text(buf.trim().to_string());
            new_message
                .send_message("127.0.0.1:8080")
                .context("Failed to send message to the server")?;
        }
    }

    Ok(())
}
