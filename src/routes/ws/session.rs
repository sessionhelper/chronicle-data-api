//! Per-connection state machine.
//!
//! Owns: the split WebSocket halves, the `SubscriptionSet`, the
//! drop-oldest queue, the heartbeat timers. Emits: every matching event to
//! the socket, periodic pings.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{interval, Instant, MissedTickBehavior};
use tracing::{debug, info, warn};

use super::filter::{Filter, SubscriptionSet};
use super::queue::DropOldestQueue;
use crate::routes::AppState;

/// How often we ping the peer.
const PING_PERIOD: Duration = Duration::from_secs(30);
/// How long we wait for a pong before dropping.
const PONG_GRACE: Duration = Duration::from_secs(10);

/// Client → server frame.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Subscribe {
        events: Vec<String>,
        #[serde(default)]
        filter: Filter,
    },
    Unsubscribe {
        events: Vec<String>,
    },
    Ping {},
}

/// Server → client control frame (events are serialized directly).
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame {
    Subscribed {
        active_filters: Vec<super::filter::Subscription>,
    },
    Unsubscribed {
        active_filters: Vec<super::filter::Subscription>,
    },
    Error {
        message: String,
    },
}

/// Why the connection closed — used in tracing + the WARN on sustained drops.
#[derive(Debug)]
#[allow(dead_code, clippy::enum_variant_names)]
enum Closed {
    PeerClosed,
    PeerError(String),
    SendFailed(String),
    InvalidFrame(String),
    HealthcheckTimeout,
    DropBurst,
}

/// Drain the broadcast bus + own socket I/O.
pub async fn run(socket: WebSocket, state: AppState, service_name: String) {
    state
        .metrics
        .set_gauge("chronicle_ws_subscribers", 1);
    let reason = run_inner(socket, &state, &service_name).await;
    state
        .metrics
        .set_gauge("chronicle_ws_subscribers", 0);
    debug!(service = %service_name, ?reason, "ws disconnected");
}

async fn run_inner(socket: WebSocket, state: &AppState, service_name: &str) -> Closed {
    let (mut tx_sock, mut rx_sock) = socket.split();
    let mut subs = SubscriptionSet::new();
    let mut q = DropOldestQueue::new(state.ws_queue_depth);
    let mut event_rx = state.events.subscribe();

    let mut ping_timer = interval(PING_PERIOD);
    ping_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut pong_deadline: Option<Instant> = None;

    // Burst detection: 10 drops / minute on a single subscriber → drop the
    // connection so the peer reconciles via REST.
    let mut drop_window_start = Instant::now();
    let mut drops_in_window: u64 = 0;

    loop {
        // Send the next queued event, if any, greedily. This is fine inside
        // the select because send is very fast on a healthy socket.
        if let Some(ev) = q.pop() {
            let json = match serde_json::to_string(&ev) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "ws serialize failed");
                    continue;
                }
            };
            // Sampling: observe ws_send only 1/100 to avoid volume blowup.
            let sample = rand::random::<u8>() < 3; // ≈ 1/85
            let t0 = std::time::Instant::now();
            if let Err(e) = tx_sock.send(Message::text(json)).await {
                return Closed::SendFailed(e.to_string());
            }
            if sample {
                let sec = t0.elapsed().as_secs_f64();
                state.metrics.observe_histogram("chronicle_ws_send_seconds", sec);
            }
            state.metrics.inc_counter("chronicle_ws_events_sent_total", 1);
        }

        tokio::select! {
            biased;

            // Incoming broadcast event: apply subscriber filter, enqueue.
            recv = event_rx.recv() => match recv {
                Ok(event) => {
                    if !subs.matches(&event) {
                        continue;
                    }
                    let overflowed = q.push(event);
                    if overflowed {
                        drops_in_window += 1;
                        state.metrics.inc_counter("chronicle_ws_events_dropped_total", 1);
                        warn!(service = %service_name, "ws queue overflow — dropped oldest");
                        if drop_window_start.elapsed() > Duration::from_secs(60) {
                            drop_window_start = Instant::now();
                            drops_in_window = 1;
                        }
                        if drops_in_window > 10 {
                            info!(service = %service_name, "ws drop burst > 10/min; disconnecting");
                            return Closed::DropBurst;
                        }
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    // Broadcast channel dropped N messages before we got a chance
                    // to read. Count them as drops and continue.
                    state.metrics.inc_counter("chronicle_ws_events_dropped_total", n);
                    warn!(service = %service_name, skipped = n, "ws broadcast lagged");
                }
                Err(RecvError::Closed) => return Closed::PeerClosed,
            },

            // Incoming socket message from the peer.
            msg = rx_sock.next() => match msg {
                None => return Closed::PeerClosed,
                Some(Err(e)) => return Closed::PeerError(e.to_string()),
                Some(Ok(m)) => match m {
                    Message::Text(text) => {
                        match serde_json::from_str::<ClientFrame>(&text) {
                            Ok(ClientFrame::Subscribe { events, filter }) => {
                                subs.subscribe(events, filter);
                                let reply = ServerFrame::Subscribed {
                                    active_filters: subs.active(),
                                };
                                let _ = tx_sock.send(Message::text(
                                    serde_json::to_string(&reply).unwrap_or_default(),
                                )).await;
                            }
                            Ok(ClientFrame::Unsubscribe { events }) => {
                                subs.unsubscribe(events);
                                let reply = ServerFrame::Unsubscribed {
                                    active_filters: subs.active(),
                                };
                                let _ = tx_sock.send(Message::text(
                                    serde_json::to_string(&reply).unwrap_or_default(),
                                )).await;
                            }
                            Ok(ClientFrame::Ping {}) => {
                                let _ = tx_sock.send(Message::Pong(vec![].into())).await;
                            }
                            Err(e) => {
                                let reply = ServerFrame::Error {
                                    message: format!("invalid frame: {e}"),
                                };
                                let _ = tx_sock.send(Message::text(
                                    serde_json::to_string(&reply).unwrap_or_default(),
                                )).await;
                            }
                        }
                    }
                    Message::Pong(_) => { pong_deadline = None; }
                    Message::Ping(p) => { let _ = tx_sock.send(Message::Pong(p)).await; }
                    Message::Close(_) => return Closed::PeerClosed,
                    Message::Binary(_) => {
                        let reply = ServerFrame::Error {
                            message: "binary frames unsupported".into(),
                        };
                        let _ = tx_sock.send(Message::text(
                            serde_json::to_string(&reply).unwrap_or_default(),
                        )).await;
                        return Closed::InvalidFrame("binary".into());
                    }
                },
            },

            // Heartbeat tick.
            _ = ping_timer.tick() => {
                if tx_sock.send(Message::Ping(vec![].into())).await.is_err() {
                    return Closed::PeerClosed;
                }
                pong_deadline = Some(Instant::now() + PONG_GRACE);
            },

            // Pong deadline — fires PONG_GRACE after each ping unless the
            // peer responded and cleared it first.
            _ = wait_deadline(pong_deadline) => {
                return Closed::HealthcheckTimeout;
            }
        }
    }
}

async fn wait_deadline(d: Option<Instant>) {
    match d {
        None => std::future::pending::<()>().await,
        Some(d) => tokio::time::sleep_until(d).await,
    }
}
