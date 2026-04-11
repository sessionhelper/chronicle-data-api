//! WebSocket endpoint for real-time event streaming.
//!
//! Clients connect to `GET /ws?token=<bearer_token>`, then send JSON
//! subscription messages to choose which events they receive. The server
//! filters the global event bus by the client's active subscriptions and
//! forwards matching events as JSON text frames.
//!
//! # Auth
//!
//! The `token` query parameter is validated against the `service_sessions`
//! table — the same check the HTTP auth middleware does. Connections
//! without a valid token are rejected with 401 before the upgrade.
//!
//! # Subscriber types
//!
//! - **Internal** (token belongs to a service session): events are queued
//!   via `tokio::sync::mpsc` with a 1000-message buffer so slow consumers
//!   don't lose events.
//! - **External** (any other valid token): events are delivered via the
//!   broadcast channel — dropped if the consumer can't keep up.
//!
//! # Subscription protocol
//!
//! ```json
//! {"subscribe": "sessions/<uuid>"}   // events for one session
//! {"subscribe": "sessions"}           // all session events
//! {"unsubscribe": "sessions/<uuid>"} // stop receiving
//! ```

use std::collections::HashSet;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use uuid::Uuid;

use crate::auth::middleware::hash_session_token;
use crate::routes::AppState;

/// Whether the connected client is an internal service or an external browser.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SubscriberType {
    /// Service session — gets reliable mpsc delivery.
    Internal,
    /// Browser / other — gets broadcast (drop on lag).
    External,
}

/// Client-to-server control message.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ClientMessage {
    Subscribe { subscribe: String },
    Unsubscribe { unsubscribe: String },
}

/// Query parameters on the WS upgrade request.
#[derive(Debug, Deserialize)]
struct WsParams {
    token: String,
}

/// Parsed topic from a subscription string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Topic {
    /// All events for all sessions.
    AllSessions,
    /// Events for a specific session.
    Session(Uuid),
}

fn parse_topic(raw: &str) -> Option<Topic> {
    if raw == "sessions" {
        return Some(Topic::AllSessions);
    }
    if let Some(id_str) = raw.strip_prefix("sessions/") {
        if let Ok(id) = id_str.parse::<Uuid>() {
            return Some(Topic::Session(id));
        }
    }
    None
}

/// Known internal service names. Tokens belonging to these services get
/// reliable mpsc-based delivery instead of broadcast.
const INTERNAL_SERVICES: &[&str] = &["chronicle-worker", "chronicle-bot", "chronicle-feeder"];

/// Validate the bearer token and determine subscriber type.
///
/// Returns `None` if the token is invalid or expired, otherwise the
/// subscriber type based on the service_name in the session row.
async fn validate_ws_token(pool: &sqlx::PgPool, token: &str) -> Option<SubscriberType> {
    let token_hash = hash_session_token(token);

    #[derive(sqlx::FromRow)]
    struct Row {
        service_name: String,
    }

    let row = sqlx::query_as::<_, Row>(
        "SELECT service_name FROM service_sessions WHERE token_hash = $1 AND alive = true",
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    let sub_type = if INTERNAL_SERVICES.contains(&row.service_name.as_str()) {
        SubscriberType::Internal
    } else {
        SubscriberType::External
    };

    tracing::debug!(
        service = %row.service_name,
        subscriber_type = ?sub_type,
        "ws token validated"
    );

    Some(sub_type)
}

async fn ws_upgrade(
    State(state): State<AppState>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Validate the token before upgrading the connection.
    let Some(subscriber_type) = validate_ws_token(&state.pool, &params.token).await else {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "invalid or expired session token",
        )
            .into_response();
    };

    ws.on_upgrade(move |socket| handle_socket(socket, state, subscriber_type))
        .into_response()
}

async fn handle_socket(socket: WebSocket, state: AppState, subscriber_type: SubscriberType) {
    let (mut sender, mut receiver) = socket.split();
    let mut subscriptions: HashSet<Topic> = HashSet::new();

    // Server heartbeat: ping every 30s. If the client doesn't respond
    // the connection will be dropped by the underlying transport.
    let mut ping_interval = interval(Duration::from_secs(30));

    match subscriber_type {
        SubscriberType::Internal => {
            // Internal services get reliable mpsc delivery with a 1000-message buffer.
            // A background task drains the broadcast bus into the mpsc channel.
            let (mpsc_tx, mut mpsc_rx) = mpsc::channel(1000);
            let mut event_rx = state.events.subscribe();

            let drain_handle = tokio::spawn(async move {
                loop {
                    match event_rx.recv().await {
                        Ok(event) => {
                            if mpsc_tx.send(event).await.is_err() {
                                // Receiver dropped — socket closed
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(skipped = n, "internal ws drain lagged");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            loop {
                tokio::select! {
                    msg = receiver.next() => {
                        if !handle_client_msg(msg, &mut subscriptions) {
                            break;
                        }
                    }

                    Some(api_event) = mpsc_rx.recv() => {
                        if topic_matches(&subscriptions, &api_event) {
                            if let Ok(json) = serde_json::to_string(&api_event) {
                                if sender.send(Message::text(json)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }

                    _ = ping_interval.tick() => {
                        if sender.send(Message::Ping(vec![].into())).await.is_err() {
                            break;
                        }
                    }
                }
            }

            drain_handle.abort();
        }

        SubscriberType::External => {
            // External subscribers use broadcast directly — drop on lag is acceptable.
            let mut event_rx = state.events.subscribe();

            loop {
                tokio::select! {
                    msg = receiver.next() => {
                        if !handle_client_msg(msg, &mut subscriptions) {
                            break;
                        }
                    }

                    event = event_rx.recv() => {
                        match event {
                            Ok(api_event) => {
                                if topic_matches(&subscriptions, &api_event) {
                                    if let Ok(json) = serde_json::to_string(&api_event) {
                                        if sender.send(Message::text(json)).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(skipped = n, "external ws client lagged behind event bus");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                break;
                            }
                        }
                    }

                    _ = ping_interval.tick() => {
                        if sender.send(Message::Ping(vec![].into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }

    tracing::debug!(?subscriber_type, "ws client disconnected");
}

/// Process a client WebSocket message. Returns `false` if the connection should close.
fn handle_client_msg(
    msg: Option<Result<Message, axum::Error>>,
    subscriptions: &mut HashSet<Topic>,
) -> bool {
    match msg {
        Some(Ok(Message::Text(text))) => {
            if let Ok(client_msg) = serde_json::from_str::<ClientMessage>(&text) {
                match client_msg {
                    ClientMessage::Subscribe { subscribe } => {
                        if let Some(topic) = parse_topic(&subscribe) {
                            tracing::debug!(?topic, "ws client subscribed");
                            subscriptions.insert(topic);
                        }
                    }
                    ClientMessage::Unsubscribe { unsubscribe } => {
                        if let Some(topic) = parse_topic(&unsubscribe) {
                            tracing::debug!(?topic, "ws client unsubscribed");
                            subscriptions.remove(&topic);
                        }
                    }
                }
            }
            true
        }
        Some(Ok(Message::Close(_))) | None => false,
        Some(Err(e)) => {
            tracing::debug!(error = %e, "ws receive error");
            false
        }
        // Pong, Binary, Ping — ignore
        _ => true,
    }
}

/// Check if an event matches any active subscription.
fn topic_matches(subscriptions: &HashSet<Topic>, event: &crate::events::ApiEvent) -> bool {
    subscriptions.contains(&Topic::AllSessions)
        || subscriptions.contains(&Topic::Session(event.session_id()))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/ws", get(ws_upgrade))
}
