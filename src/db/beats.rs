use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Beat {
    pub id: Uuid,
    pub session_id: Uuid,
    pub beat_index: i32,
    pub start_time: f64,
    pub end_time: f64,
    pub title: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBeat {
    pub beat_index: i32,
    pub start_time: f64,
    pub end_time: f64,
    pub title: String,
    #[serde(default)]
    pub summary: String,
}

pub async fn bulk_create(
    pool: &PgPool,
    session_id: Uuid,
    beats: &[CreateBeat],
) -> Result<Vec<Beat>, AppError> {
    let mut results = Vec::with_capacity(beats.len());

    for beat in beats {
        let row = sqlx::query_as::<_, Beat>(
            "INSERT INTO session_beats (session_id, beat_index, start_time, end_time, title, summary)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING *"
        )
        .bind(session_id)
        .bind(beat.beat_index)
        .bind(beat.start_time)
        .bind(beat.end_time)
        .bind(&beat.title)
        .bind(&beat.summary)
        .fetch_one(pool)
        .await?;

        results.push(row);
    }

    Ok(results)
}

pub async fn list(pool: &PgPool, session_id: Uuid) -> Result<Vec<Beat>, AppError> {
    let rows = sqlx::query_as::<_, Beat>(
        "SELECT * FROM session_beats WHERE session_id = $1 ORDER BY beat_index"
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
