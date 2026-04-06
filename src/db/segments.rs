use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Segment {
    pub id: Uuid,
    pub session_id: Uuid,
    pub segment_index: i32,
    pub speaker_pseudo_id: String,
    pub start_time: f64,
    pub end_time: f64,
    pub text: String,
    pub original_text: String,
    pub confidence: Option<f64>,
    pub beat_id: Option<i32>,
    pub chunk_group: Option<i32>,
    pub excluded: bool,
    pub exclude_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSegment {
    pub segment_index: i32,
    pub speaker_pseudo_id: String,
    pub start_time: f64,
    pub end_time: f64,
    pub text: String,
    pub original_text: String,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub beat_id: Option<i32>,
    #[serde(default)]
    pub chunk_group: Option<i32>,
    #[serde(default)]
    pub excluded: bool,
    #[serde(default)]
    pub exclude_reason: Option<String>,
}

pub async fn bulk_create(
    pool: &PgPool,
    session_id: Uuid,
    segments: &[CreateSegment],
) -> Result<Vec<Segment>, AppError> {
    let mut results = Vec::with_capacity(segments.len());

    for seg in segments {
        let row = sqlx::query_as::<_, Segment>(
            "INSERT INTO transcript_segments (session_id, segment_index, speaker_pseudo_id, start_time, end_time, text, original_text, confidence, beat_id, chunk_group, excluded, exclude_reason)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             RETURNING *"
        )
        .bind(session_id)
        .bind(seg.segment_index)
        .bind(&seg.speaker_pseudo_id)
        .bind(seg.start_time)
        .bind(seg.end_time)
        .bind(&seg.text)
        .bind(&seg.original_text)
        .bind(seg.confidence)
        .bind(seg.beat_id)
        .bind(seg.chunk_group)
        .bind(seg.excluded)
        .bind(&seg.exclude_reason)
        .fetch_one(pool)
        .await?;

        results.push(row);
    }

    Ok(results)
}

pub async fn list(pool: &PgPool, session_id: Uuid) -> Result<Vec<Segment>, AppError> {
    let rows = sqlx::query_as::<_, Segment>(
        "SELECT * FROM transcript_segments WHERE session_id = $1 ORDER BY segment_index"
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
