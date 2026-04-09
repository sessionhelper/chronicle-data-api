use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: Uuid,
    pub guild_id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub game_system: Option<String>,
    pub campaign_name: Option<String>,
    pub participant_count: Option<i32>,
    pub s3_prefix: Option<String>,
    pub title: Option<String>,
    pub status: String,
    pub collaborative_editing: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSession {
    pub id: Uuid,
    pub guild_id: i64,
    pub started_at: DateTime<Utc>,
    pub game_system: Option<String>,
    pub campaign_name: Option<String>,
    pub s3_prefix: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSession {
    pub ended_at: Option<DateTime<Utc>>,
    pub status: Option<String>,
    pub participant_count: Option<i32>,
    pub game_system: Option<String>,
    pub campaign_name: Option<String>,
    pub title: Option<String>,
}

pub async fn create(pool: &PgPool, input: &CreateSession) -> Result<Session, AppError> {
    let row = sqlx::query_as::<_, Session>(
        "INSERT INTO sessions (id, guild_id, started_at, game_system, campaign_name, s3_prefix, title)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING *"
    )
    .bind(input.id)
    .bind(input.guild_id)
    .bind(input.started_at)
    .bind(&input.game_system)
    .bind(&input.campaign_name)
    .bind(&input.s3_prefix)
    .bind(&input.title)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Session, AppError> {
    let row = sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("session {id} not found")))?;

    Ok(row)
}

pub async fn update(pool: &PgPool, id: Uuid, input: &UpdateSession) -> Result<Session, AppError> {
    let row = sqlx::query_as::<_, Session>(
        "UPDATE sessions SET
            ended_at = COALESCE($2, ended_at),
            status = COALESCE($3, status),
            participant_count = COALESCE($4, participant_count),
            game_system = COALESCE($5, game_system),
            campaign_name = COALESCE($6, campaign_name),
            title = COALESCE($7, title)
         WHERE id = $1
         RETURNING *"
    )
    .bind(id)
    .bind(input.ended_at)
    .bind(&input.status)
    .bind(input.participant_count)
    .bind(&input.game_system)
    .bind(&input.campaign_name)
    .bind(&input.title)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("session {id} not found")))?;

    Ok(row)
}

/// List all sessions in a given status, ordered oldest-first. The worker
/// polls this with `status=uploaded` to find sessions ready to transcribe;
/// oldest-first means FIFO processing without needing a separate queue.
pub async fn list_by_status(pool: &PgPool, status: &str) -> Result<Vec<Session>, AppError> {
    let rows = sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions WHERE status = $1 ORDER BY started_at ASC"
    )
    .bind(status)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn list_by_user(pool: &PgPool, user_pseudo_id: &str) -> Result<Vec<Session>, AppError> {
    let rows = sqlx::query_as::<_, Session>(
        "SELECT s.* FROM sessions s
         JOIN session_participants sp ON sp.session_id = s.id
         JOIN users u ON u.id = sp.user_id
         WHERE u.pseudo_id = $1
         ORDER BY s.started_at DESC"
    )
    .bind(user_pseudo_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
