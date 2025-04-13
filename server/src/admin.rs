//! Admin web interface module for the CrabChat server application.
//!
//! This module provides a web-based admin interface for managing users and messages
//! in the CrabChat system, including viewing logs, filtering messages, and deleting users.

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Postgres};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tracing::{error, info};

use utils::MessageType;

/// Configuration for the admin web server.
pub struct AdminConfig {
    pub hostname: String,
    pub port: u16,
}

/// Shared state between the admin interface and the chat server.
pub struct AdminState {
    pub db_pool: Pool<Postgres>,
    pub clients: Arc<Mutex<HashMap<std::net::SocketAddr, super::ClientInfo>>>,
    pub sender: broadcast::Sender<(MessageType, std::net::SocketAddr)>,
}

/// Filter parameters for message queries
#[derive(Debug, Deserialize, Default)]
pub struct MessageFilter {
    user_id: Option<i64>,
    message_type: Option<String>,
    limit: Option<i64>,
}

/// User data structure for admin views
#[derive(Debug, Serialize, FromRow)]
struct UserData {
    id: i64,
    username: String,
    message_count: i64,
    first_seen: chrono::DateTime<chrono::Utc>,
    #[sqlx(default)]
    online: bool,
}

/// Message data structure for admin views
#[derive(Debug, Serialize, FromRow)]
struct MessageData {
    id: i64,
    user_id: i64,
    username: String,
    message_type: String,
    content: String,
    sent_at: chrono::DateTime<chrono::Utc>,
}

/// Starts the admin web interface.
///
/// This function sets up routes and starts the Axum server for the admin interface.
///
/// # Arguments
/// - `config`: The configuration for the admin web server.
/// - `admin_state`: The shared state between the admin interface and the chat server.
///
/// # Errors
/// Returns an error if the server fails to start.
pub async fn start_admin_interface(
    config: AdminConfig,
    admin_state: Arc<AdminState>,
) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(admin_home))
        .route("/users", get(list_users))
        .route("/users/{id}/delete", post(delete_user))
        .route("/messages", get(list_messages))
        .route("/messages/{id}/delete", post(delete_message))
        .with_state(admin_state);

    let addr = format!("{}:{}", config.hostname, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    info!("Admin server listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}

/// Handler for the admin home page.
async fn admin_home() -> impl IntoResponse {
    Html(
        r#"
        <!DOCTYPE html>
        <html>
        <head>
            <title>CrabChat Admin</title>
            <style>
                body { font-family: Arial, sans-serif; margin: 0; padding: 20px; }
                nav { margin-bottom: 20px; }
                nav a { margin-right: 15px; }
                h1 { color: #333; }
            </style>
        </head>
        <body>
            <h1>CrabChat Admin Interface</h1>
            <nav>
                <a href="/users">Manage Users</a>
                <a href="/messages">View Messages</a>
            </nav>
            <p>Welcome to the CrabChat admin interface. Use the links above to manage users and view messages.</p>
        </body>
        </html>
        "#,
    )
}

/// Handler to list all users with message counts.
async fn list_users(State(state): State<Arc<AdminState>>) -> impl IntoResponse {
    match get_users_with_message_count(&state).await {
        Ok(users) => {
            let clients = state.clients.lock().await;

            let mut rows = String::new();
            for user in users {
                let online = clients.values().any(|c| c.user_id == user.id);
                rows.push_str(&format!(
                    r#"
                    <tr>
                        <td>{}</td>
                        <td>{}</td>
                        <td>{}</td>
                        <td>{}</td>
                        <td>{}</td>
                        <td>
                            <form action="/users/{}/delete" method="post" onsubmit="return confirm('Are you sure you want to delete this user and all their messages?');">
                                <button type="submit">Delete</button>
                            </form>
                        </td>
                        <td>
                            <a href="/messages?user_id={}">View Messages</a>
                        </td>
                    </tr>
                    "#,
                    user.id, user.username, user.message_count,
                    user.first_seen.format("%Y-%m-%d %H:%M:%S"),
                    if online { "Yes" } else { "No" }, user.id, user.id
                ));
            }

            Html(format!(
                r#"
                <!DOCTYPE html>
                <html>
                <head>
                    <title>CrabChat Admin - Users</title>
                    <style>
                        body {{ font-family: Arial, sans-serif; margin: 0; padding: 20px; }}
                        table {{ border-collapse: collapse; width: 100%; }}
                        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
                        tr:nth-child(even) {{ background-color: #f2f2f2; }}
                        th {{ padding-top: 12px; padding-bottom: 12px; background-color: #4CAF50; color: white; }}
                        .nav {{ margin-bottom: 20px; }}
                        .nav a {{ margin-right: 15px; }}
                        h1 {{ color: #333; }}
                    </style>
                </head>
                <body>
                    <div class="nav">
                        <a href="/">Home</a>
                        <a href="/messages">Messages</a>
                    </div>
                    <h1>Users</h1>
                    <table>
                        <thead>
                            <tr>
                                <th>ID</th>
                                <th>Username</th>
                                <th>Message Count</th>
                                <th>First Seen</th>
                                <th>Online</th>
                                <th>Actions</th>
                                <th>Messages</th>
                            </tr>
                        </thead>
                        <tbody>
                            {}
                        </tbody>
                    </table>
                </body>
                </html>
                "#,
                rows
            ))
        }
        Err(e) => {
            error!("Failed to retrieve users: {}", e);
            Html(format!("<p>Failed to retrieve users: {}</p>", e))
        }
    }
}

/// Handler to delete a user and all their messages.
async fn delete_user(
    State(state): State<Arc<AdminState>>,
    Path(user_id): Path<i64>,
) -> impl IntoResponse {
    match delete_user_by_id(&state, user_id).await {
        Ok(_) => {
            // Also send a broadcast to notify all clients
            let broadcast_msg =
                MessageType::Text("[Admin: A user has been removed from the system]".to_string());
            let _ = state
                .sender
                .send((broadcast_msg, std::net::SocketAddr::from(([0, 0, 0, 0], 0))));

            Redirect::to("/users").into_response()
        }
        Err(e) => {
            error!("Failed to delete user {}: {}", user_id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("<p>Failed to delete user: {}</p>", e)),
            )
                .into_response()
        }
    }
}

/// Handler to list all messages with optional filtering.
async fn list_messages(
    State(state): State<Arc<AdminState>>,
    Query(filter): Query<MessageFilter>,
) -> impl IntoResponse {
    match get_messages(&state, &filter).await {
        Ok(messages) => {
            let mut rows = String::new();
            for msg in messages {
                // Safely truncate content if it's too long for display
                let display_content = if msg.content.len() > 100 {
                    format!("{}...", &msg.content[..97])
                } else {
                    msg.content.clone()
                };

                rows.push_str(&format!(
                    r#"
                    <tr>
                        <td>{}</td>
                        <td>{}</td>
                        <td>{}</td>
                        <td>{}</td>
                        <td>{}</td>
                        <td>{}</td>
                        <td>
                            <form action="/messages/{}/delete" method="post" onsubmit="return confirm('Are you sure you want to delete this message?');">
                                <button type="submit">Delete</button>
                            </form>
                        </td>
                    </tr>
                    "#,
                    msg.id, msg.user_id, msg.username,
                    msg.message_type, display_content,
                    msg.sent_at.format("%Y-%m-%d %H:%M:%S"),
                    msg.id
                ));
            }

            // Build filter form
            let user_filter = match get_users_for_filter(&state).await {
                Ok(users) => {
                    let mut options = String::new();
                    options.push_str(r#"<option value="">All Users</option>"#);

                    for user in users {
                        let selected = filter.user_id == Some(user.id);
                        options.push_str(&format!(
                            r#"<option value="{}" {}>{}</option>"#,
                            user.id,
                            if selected { "selected" } else { "" },
                            user.username
                        ));
                    }

                    format!(
                        r#"
                        <label for="user_id">User:</label>
                        <select name="user_id" id="user_id">
                            {}
                        </select>
                        "#,
                        options
                    )
                }
                Err(e) => {
                    error!("Failed to get users for filter: {}", e);
                    String::new()
                }
            };

            Html(format!(
                r#"
                <!DOCTYPE html>
                <html>
                <head>
                    <title>CrabChat Admin - Messages</title>
                    <style>
                        body {{ font-family: Arial, sans-serif; margin: 0; padding: 20px; }}
                        table {{ border-collapse: collapse; width: 100%; }}
                        th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
                        tr:nth-child(even) {{ background-color: #f2f2f2; }}
                        th {{ padding-top: 12px; padding-bottom: 12px; background-color: #4CAF50; color: white; }}
                        .nav {{ margin-bottom: 20px; }}
                        .nav a {{ margin-right: 15px; }}
                        h1 {{ color: #333; }}
                        .filters {{ margin-bottom: 20px; padding: 10px; background-color: #f9f9f9; border: 1px solid #ddd; }}
                        .filters label {{ margin-right: 10px; }}
                        .filters select, .filters input {{ margin-right: 20px; }}
                    </style>
                </head>
                <body>
                    <div class="nav">
                        <a href="/">Home</a>
                        <a href="/users">Users</a>
                    </div>
                    <h1>Messages</h1>
                    
                    <div class="filters">
                        <form action="/messages" method="get">
                            {}
                            <label for="message_type">Type:</label>
                            <select name="message_type" id="message_type">
                                <option value="">All Types</option>
                                <option value="Text" {}">Text</option>
                                <option value="File" {}">File</option>
                                <option value="Image" {}">Image</option>
                            </select>
                            
                            <label for="limit">Limit:</label>
                            <input type="number" name="limit" id="limit" value="{}" min="1" max="1000">
                            
                            <button type="submit">Filter</button>
                        </form>
                    </div>
                    
                    <table>
                        <thead>
                            <tr>
                                <th>ID</th>
                                <th>User ID</th>
                                <th>Username</th>
                                <th>Type</th>
                                <th>Content</th>
                                <th>Sent At</th>
                                <th>Actions</th>
                            </tr>
                        </thead>
                        <tbody>
                            {}
                        </tbody>
                    </table>
                </body>
                </html>
                "#,
                user_filter,
                if filter.message_type.as_deref() == Some("Text") { "selected" } else { "" },
                if filter.message_type.as_deref() == Some("File") { "selected" } else { "" },
                if filter.message_type.as_deref() == Some("Image") { "selected" } else { "" },
                filter.limit.unwrap_or(100),
                rows
            )).into_response()
        }
        Err(e) => {
            error!("Failed to retrieve messages: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("<p>Failed to retrieve messages</p>".to_string()),
            )
                .into_response()
        }
    }
}

/// Handler to delete a single message.
async fn delete_message(
    State(state): State<Arc<AdminState>>,
    Path(message_id): Path<i64>,
) -> impl IntoResponse {
    match delete_message_by_id(&state, message_id).await {
        Ok(_) => Redirect::to("/messages").into_response(),
        Err(e) => {
            error!("Failed to delete message {}: {}", message_id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(format!("<p>Failed to delete message: {}</p>", e)),
            )
                .into_response()
        }
    }
}

// --- Database Operations ---

/// Retrieves a list of all users with their message counts.
async fn get_users_with_message_count(state: &AdminState) -> Result<Vec<UserData>, sqlx::Error> {
    let users = sqlx::query_as!(
        UserData,
        r#"
        SELECT u.id, u.username, COUNT(m.id) as "message_count!: i64", u.first_seen as "first_seen!", false as "online!: bool"
        FROM users u
        LEFT JOIN messages m ON u.id = m.user_id
        GROUP BY u.id, u.first_seen
        ORDER BY u.id
        "#
    )
    .fetch_all(&state.db_pool)
    .await?;

    Ok(users)
}

/// Retrieves a simplified list of users for filter dropdowns.
async fn get_users_for_filter(state: &AdminState) -> Result<Vec<UserData>, sqlx::Error> {
    let users = sqlx::query_as!(
        UserData,
        r#"
        SELECT id, username, 0 as "message_count!: i64", first_seen, false as "online!: bool"
        FROM users
        ORDER BY username
        "#
    )
    .fetch_all(&state.db_pool)
    .await?;

    Ok(users)
}

/// Retrieves messages with optional filtering.
async fn get_messages(
    state: &AdminState,
    filter: &MessageFilter,
) -> Result<Vec<MessageData>, sqlx::Error> {
    // Base query with static parts
    let mut query_builder = sqlx::QueryBuilder::new(
        "SELECT m.id, m.user_id, u.username, m.message_type, m.content, m.sent_at 
         FROM messages m
         JOIN users u ON m.user_id = u.id
         WHERE 1=1",
    );

    // Add optional filters
    if let Some(user_id) = filter.user_id {
        query_builder.push(" AND m.user_id = ");
        query_builder.push_bind(user_id);
    }

    if let Some(msg_type) = &filter.message_type {
        query_builder.push(" AND m.message_type = ");
        query_builder.push_bind(msg_type);
    }

    // Add order by
    query_builder.push(" ORDER BY m.sent_at DESC LIMIT ");
    query_builder.push_bind(filter.limit.unwrap_or(100));

    // Build and execute the query
    let query = query_builder.build_query_as::<MessageData>();
    query.fetch_all(&state.db_pool).await
}

/// Deletes a user and all their messages.
async fn delete_user_by_id(state: &AdminState, user_id: i64) -> Result<(), sqlx::Error> {
    let mut tx = state.db_pool.begin().await?;

    // First delete all messages from this user
    sqlx::query!("DELETE FROM messages WHERE user_id = $1", user_id)
        .execute(&mut *tx)
        .await?;

    // Then delete the user
    sqlx::query!("DELETE FROM users WHERE id = $1", user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    info!("Deleted user and all messages for user_id: {}", user_id);
    Ok(())
}

/// Deletes a single message.
async fn delete_message_by_id(state: &AdminState, message_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM messages WHERE id = $1", message_id)
        .execute(&state.db_pool)
        .await?;

    info!("Deleted message_id: {}", message_id);
    Ok(())
}
