//! Internal event bus for real-time notifications.
//!
//! Every mutation in the API broadcasts an event on a
//! `tokio::sync::broadcast` channel. WebSocket clients subscribe
//! to the bus and filter by session topic.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

/// A domain event emitted after a successful mutation.
///
/// Tagged-enum serialization gives each variant a stable `"event"` key
/// for downstream consumers (WS clients, SSE bridges) to switch on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum ApiEvent {
    #[serde(rename = "session_status_changed")]
    SessionStatusChanged {
        session_id: Uuid,
        status: String,
    },

    #[serde(rename = "chunk_uploaded")]
    ChunkUploaded {
        session_id: Uuid,
        pseudo_id: String,
        seq: u32,
        size: usize,
    },

    #[serde(rename = "segment_added")]
    SegmentAdded {
        session_id: Uuid,
        segment: serde_json::Value,
    },

    #[serde(rename = "segments_batch_added")]
    SegmentsBatchAdded {
        session_id: Uuid,
        count: usize,
    },

    #[serde(rename = "beat_detected")]
    BeatDetected {
        session_id: Uuid,
        beat: serde_json::Value,
    },

    #[serde(rename = "scene_detected")]
    SceneDetected {
        session_id: Uuid,
        scene: serde_json::Value,
    },

    #[serde(rename = "transcription_progress")]
    TranscriptionProgress {
        session_id: Uuid,
        stage: String,
        detail: String,
    },
}

impl ApiEvent {
    /// The session this event belongs to, used for topic filtering.
    pub fn session_id(&self) -> Uuid {
        match self {
            Self::SessionStatusChanged { session_id, .. }
            | Self::ChunkUploaded { session_id, .. }
            | Self::SegmentAdded { session_id, .. }
            | Self::SegmentsBatchAdded { session_id, .. }
            | Self::BeatDetected { session_id, .. }
            | Self::SceneDetected { session_id, .. }
            | Self::TranscriptionProgress { session_id, .. } => *session_id,
        }
    }
}

pub type EventSender = broadcast::Sender<ApiEvent>;

/// Create a new broadcast channel for API events.
///
/// The capacity of 1024 is generous — slow consumers that fall behind
/// will get a `RecvError::Lagged` and can reconnect or skip.
pub fn create_event_bus() -> EventSender {
    let (tx, _) = broadcast::channel(1024);
    tx
}
