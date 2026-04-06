use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Scene {
    pub id: Uuid,
    pub session_id: Uuid,
    pub scene_index: i32,
    pub start_time: f64,
    pub end_time: f64,
    pub title: String,
    pub summary: String,
    pub beat_start: i32,
    pub beat_end: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateScene {
    pub scene_index: i32,
    pub start_time: f64,
    pub end_time: f64,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub beat_start: i32,
    pub beat_end: i32,
}

pub async fn bulk_create(
    pool: &PgPool,
    session_id: Uuid,
    scenes: &[CreateScene],
) -> Result<Vec<Scene>, AppError> {
    let mut results = Vec::with_capacity(scenes.len());

    for scene in scenes {
        let row = sqlx::query_as::<_, Scene>(
            "INSERT INTO session_scenes (session_id, scene_index, start_time, end_time, title, summary, beat_start, beat_end)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             RETURNING *"
        )
        .bind(session_id)
        .bind(scene.scene_index)
        .bind(scene.start_time)
        .bind(scene.end_time)
        .bind(&scene.title)
        .bind(&scene.summary)
        .bind(scene.beat_start)
        .bind(scene.beat_end)
        .fetch_one(pool)
        .await?;

        results.push(row);
    }

    Ok(results)
}

pub async fn list(pool: &PgPool, session_id: Uuid) -> Result<Vec<Scene>, AppError> {
    let rows = sqlx::query_as::<_, Scene>(
        "SELECT * FROM session_scenes WHERE session_id = $1 ORDER BY scene_index"
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
