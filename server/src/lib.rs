use anyhow::{Context, Result as AnyhowResult};
use sqlx::{
    Pool,
    postgres::{PgPoolOptions, Postgres},
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{self as tokio_io, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, error, info, instrument, warn};
use utils::errors::{ChatError, ChatResult};

use utils::MessageType;

struct ServerState {
    clients: Mutex<HashMap<SocketAddr, ClientInfo>>,
    sender: broadcast::Sender<(MessageType, SocketAddr)>,
    db_pool: Pool<Postgres>,
    server_save_path: Arc<Path>,
}

#[derive(Debug, Clone)]
struct ClientInfo {
    username: String,
    user_id: i64,
}

// --- Database Operations ---

#[instrument(skip(pool))]
async fn get_or_insert_user(
    pool: &Pool<Postgres>,
    username: &str,
) -> Result<(i64, bool), sqlx::Error> {
    let maybe_user: Option<(i64,)> = sqlx::query_as("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_optional(pool)
        .await?;

    if let Some((id,)) = maybe_user {
        debug!("User '{}' found with ID {}", username, id);
        Ok((id, false))
    } else {
        // Use RETURNING id to get the generated ID in PostgreSQL
        let new_id: i64 =
            sqlx::query_scalar("INSERT INTO users (username) VALUES ($1) RETURNING id")
                .bind(username)
                .fetch_one(pool) // fetch_one because RETURNING guarantees a row
                .await?;
        info!("New user '{}' inserted with ID {}", username, new_id);
        Ok((new_id, true))
    }
}

#[instrument(skip(pool, message))]
async fn store_message(
    pool: &Pool<Postgres>,
    user_id: i64,
    message: &MessageType,
    save_path: &Path,
) -> Result<(), sqlx::Error> {
    let (message_type_str, content_str, raw_content_opt): (&str, String, Option<Vec<u8>>) =
        match message {
            MessageType::Text(text) => ("Text", text.clone(), None),
            MessageType::File { name, content } => {
                // Storing reference (filename) in 'content', actual binary in 'raw_content'
                ("File", name.clone(), Some(content.clone()))
            }
            MessageType::Image(content) => {
                let filename = format!("{}_{}.png", user_id, chrono::Utc::now().timestamp_millis());
                ("Image", filename, Some(content.clone()))
            }
            _ => return Ok(()), // Don't store other message types
        };

    debug!(user_id, message_type=%message_type_str, content=%content_str, has_raw=raw_content_opt.is_some(), "Storing message in DB");

    sqlx::query(
        "INSERT INTO messages (user_id, message_type, content, raw_content) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id) // $1
    .bind(message_type_str) // $2
    .bind(content_str) // $3
    .bind(raw_content_opt) // $4
    .execute(pool)
    .await?;

    Ok(())
}

// --- Server Logic ---

#[instrument(skip(state, stream), fields(client_addr = %addr))]
async fn handle_client(
    state: Arc<ServerState>,
    stream: TcpStream,
    addr: SocketAddr,
) -> ChatResult<()> {
    info!("Starting handler for client: {}", addr);

    if let Err(e) = stream.set_nodelay(true) {
        warn!("Failed to set TCP_NODELAY for {}: {}", addr, e);
    }

    let (reader, mut writer) = tokio_io::split(stream);
    let mut buf_reader = BufReader::new(reader);

    // --- Login Phase ---
    let client_info = match authenticate_client(&state, &mut buf_reader, &mut writer, addr).await {
        Ok(info) => info,
        Err(e) => {
            let error_msg = MessageType::Error(format!("Authentication failed: {}", e));
            let _ = error_msg.send(&mut writer).await;
            return Err(e);
        }
    };

    state.clients.lock().await.insert(addr, client_info.clone());
    info!(
        "Client '{}' (ID: {}) added to active clients.",
        client_info.username, client_info.user_id
    );

    // --- Subscribe to broadcast channel ---
    let mut broadcast_rx = state.sender.subscribe();

    loop {
        tokio::select! {
            result = MessageType::receive(&mut buf_reader) => {
                 let message = match result {
                    Ok(msg) => msg,
                    Err(ChatError::ConnectionClosed) => {
                        info!("Client {} connection closed (read)", addr);
                        break;
                    }
                    Err(ChatError::DatabaseError(e)) => {
                        error!("Database error during receive phase for {}: {}", addr, e);
                        break;
                     }
                    Err(e) => {
                        warn!("Error receiving message from {}: {}", addr, e);
                        let error_msg = MessageType::Error(format!("Error reading your message: {}", e));
                        if let Err(send_err) = error_msg.send(&mut writer).await {
                             error!("Failed to send error message back to {}: {}", addr, send_err);
                             break;
                         }
                         if matches!(e, ChatError::SerializationError(_) | ChatError::MessageTooLarge(_) | ChatError::EmptyMessage) {
                             continue;
                         } else {
                             break;
                         }
                    }
                };

                 match message {
                     MessageType::Text(ref text) => {
                         info!("Received Text from '{}': {}", client_info.username, text);
                         if let Err(e) = store_message(&state.db_pool, client_info.user_id, &message, &state.server_save_path).await {
                             error!("DB store failed for {}: {}", client_info.username, e);
                             let chat_db_error = ChatError::DatabaseError(e);
                             let _ = MessageType::Error(format!("Server failed to store message: {}", chat_db_error)).send(&mut writer).await;
                         } else {
                             let broadcast_msg = MessageType::Text(format!("[{}]: {}", client_info.username, text));
                             if state.sender.send((broadcast_msg, addr)).is_err() {
                                 debug!("No active listeners to broadcast to.");
                             }
                         }
                     }
                     MessageType::File{ ref name, ref content } => {
                         info!("Received File '{}' ({} bytes) from '{}'", name, content.len(), client_info.username);
                         let files_dir = state.server_save_path.join("files");
                         if let Err(e) = utils::save_binary_content(&files_dir, Some(name), content).await {
                              error!("Failed to save file '{}' from {}: {}", name, client_info.username, e);
                              let _ = MessageType::Error(format!("Failed to save file '{}' on server", name)).send(&mut writer).await;
                          } else {
                              info!("Saved file '{}' from {}", name, client_info.username);
                              if let Err(e) = store_message(&state.db_pool, client_info.user_id, &message, &state.server_save_path).await {
                                  error!("DB store failed for file from {}: {}", client_info.username, e);
                                  let _ = MessageType::Error(format!("Server failed to store file meta: {}", ChatError::DatabaseError(e))).send(&mut writer).await;
                              } else {
                                   // Broadcast notification
                                  let broadcast_msg = MessageType::Text(format!("[{} sent file: {} ({} bytes)]", client_info.username, name, content.len()));
                                   if state.sender.send((broadcast_msg, addr)).is_err() {
                                       debug!("No active listeners for file notification broadcast.");
                                   }
                              }
                          }
                     }
                     MessageType::Image(ref content) => {
                          info!("Received Image ({} bytes) from '{}'", content.len(), client_info.username);
                          let img_dir = state.server_save_path.join("images");
                          let img_filename = format!("{}_{}.png", client_info.user_id, chrono::Utc::now().timestamp_millis());
                           if let Err(e) = utils::save_binary_content(&img_dir, Some(&img_filename), content).await {
                               error!("Failed to save image from {}: {}", client_info.username, e);
                                let _ = MessageType::Error("Failed to save image on server".to_string()).send(&mut writer).await;
                           } else {
                               info!("Saved image as '{}' from {}", img_filename, client_info.username);
                               if let Err(e) = store_message(&state.db_pool, client_info.user_id, &message, &state.server_save_path).await {
                                    error!("DB store failed for image from {}: {}", client_info.username, e);
                                     let _ = MessageType::Error(format!("Server failed to store image meta: {}", ChatError::DatabaseError(e))).send(&mut writer).await;
                                } else {
                                      // Broadcast notification
                                     let broadcast_msg = MessageType::Text(format!("[{} sent an image ({} bytes)]", client_info.username, content.len()));
                                      if state.sender.send((broadcast_msg, addr)).is_err() {
                                         debug!("No active listeners for image notification broadcast.");
                                     }
                                }
                           }
                     }
                    m => warn!("Received unexpected message type from client {}: {:?}", addr, m),
                 }
            },

            result = broadcast_rx.recv() => {
                 match result {
                    Ok((message, sender_addr)) => {
                        if addr != sender_addr {
                             debug!("Forwarding broadcast message to {}", addr);
                             if let Err(e) = message.send(&mut writer).await {
                                error!("Error sending broadcast message to {}: {}", addr, e);
                                if matches!(e, ChatError::ConnectionClosed | ChatError::ConnectionError(_)) {
                                     info!("Client {} connection closed (write)", addr);
                                     break;
                                 }
                             }
                         } else {
                             debug!("Skipping broadcast to self ({})", addr);
                         }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                         error!("Broadcast channel closed unexpectedly. Exiting handler for {}.", addr);
                         break;
                     }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Client {} lagged behind {} messages. Some messages lost.", addr, n);
                     }
                } 
            } 
        } 
    } 

    // --- Cleanup ---
    info!("Handler loop finished for client {}", addr);
    let disconnect_msg = MessageType::Text(format!("[{} has disconnected]", client_info.username));
    if state.sender.send((disconnect_msg, addr)).is_err() {
        debug!("No active listeners for disconnect broadcast.");
    }

    Ok(())
}

/// Handles the initial login message exchange.
/// Uses the Pool<Postgres> from the state.
#[instrument(skip(state, reader, writer), fields(client_addr = %addr))]
async fn authenticate_client<R, W>(
    state: &Arc<ServerState>,
    reader: &mut R,
    writer: &mut W,
    addr: SocketAddr,
) -> ChatResult<ClientInfo>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    info!("Awaiting login message");

    match MessageType::receive(reader).await? {
        MessageType::Login(username) => {
            info!("Login attempt from {} with username '{}'", addr, username);
            if username.trim().is_empty() || username.len() > 50 {
                warn!("Invalid username received from {}: '{}'", addr, username);
                let _ = MessageType::Error("Invalid username format".to_string())
                    .send(writer)
                    .await;
                return Err(ChatError::AuthError("Invalid username format".to_string()));
            }

            // Check uniqueness among currently connected clients
            let clients_lock = state.clients.lock().await;
            if clients_lock.values().any(|c| c.username == username) {
                warn!(
                    "Username '{}' is already connected (from {})",
                    username, addr
                );

                let _ = MessageType::Error(format!("Username '{}' is already in use", username))
                    .send(writer)
                    .await;
                return Err(ChatError::AuthError(format!(
                    "Username '{}' is already in use",
                    username
                )));
            }
            drop(clients_lock);

            match get_or_insert_user(&state.db_pool, &username).await {
                Ok((user_id, is_new)) => {
                    info!(
                        "User '{}' (ID: {}) authenticated for {}. New user: {}",
                        username, user_id, addr, is_new
                    );
                    MessageType::LoginOk.send(writer).await?;
                    info!("Sent LoginOk to {}", addr);

                    let join_msg = MessageType::Text(format!("[{} has joined]", username));
                    if state.sender.send((join_msg, addr)).is_err() {
                        debug!("No active listeners for join broadcast.");
                    }

                    Ok(ClientInfo { username, user_id })
                }
                Err(db_err) => {
                    error!(
                        "Database error during authentication for {}: {}",
                        addr, db_err
                    );
                    let _ = MessageType::Error("Server database error during login".to_string())
                        .send(writer)
                        .await;
                    Err(ChatError::DatabaseError(db_err))
                }
            }
        }
        other => {
            warn!(
                "Expected Login message from {}, but received: {:?}",
                addr, other
            );
            let _ = MessageType::Error("Expected Login message first".to_string())
                .send(writer)
                .await;
            Err(ChatError::AuthError(
                "Expected Login message first".to_string(),
            ))
        }
    }
}

pub async fn listen_and_accept(hostname: &str, port: u16) -> AnyhowResult<()> {
    let address = format!("{}:{}", hostname, port);

    // --- Database Setup ---
    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL environment variable not set (expecting Postgres URL)")?;

    info!("Connecting to PostgreSQL database...");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .context(format!("Failed to connect to Postgres: {}", database_url))?;
    info!("Connected to PostgreSQL successfully.");

    info!("Running database migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;
    info!("Migrations complete.");

    // --- Server State Setup ---
    let save_path = Path::new("server_received").to_path_buf();
    tokio::fs::create_dir_all(save_path.join("files")).await?;
    tokio::fs::create_dir_all(save_path.join("images")).await?;
    info!("Server will save files/images to: {}", save_path.display());

    let (sender, _) = broadcast::channel(100);
    let server_state = Arc::new(ServerState {
        clients: Mutex::new(HashMap::new()),
        sender,
        db_pool: pool,
        server_save_path: Arc::from(save_path),
    });

    // --- Main Loop ---
    let listener = TcpListener::bind(&address)
        .await
        .with_context(|| format!("Failed to bind to {}", address))?;
    info!("Server listening on {}", address);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!("New connection from {}", addr);
                let server_state_clone = Arc::clone(&server_state);
                tokio::spawn(async move {
                    if let Err(e) = handle_client(server_state_clone.clone(), stream, addr).await {
                        match e {
                            ChatError::ConnectionClosed => info!("Client {} disconnected.", addr),
                            ChatError::AuthError(msg) => warn!("Auth failed for {}: {}", addr, msg),
                            ChatError::DatabaseError(dbe) => {
                                error!("Database error handling client {}: {}", addr, dbe)
                            }
                            _ => error!("Error handling client {}: {}", addr, e),
                        }
                    }
                    // Cleanup client from state
                    server_state_clone.clients.lock().await.remove(&addr);
                    info!("Cleaned up resources for client {}", addr);
                });
            }
            Err(e) => {
                error!("Error accepting connection: {}", e);
            }
        }
    }
}
