//! Chunk metadata table. `seq` is server-assigned; `client_chunk_id`
//! enforces idempotency for retried POSTs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::ids::PseudoId;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ChunkRow {
    pub session_id: Uuid,
    pub pseudo_id: PseudoId,
    pub seq: i32,
    pub s3_key: String,
    pub size_bytes: i32,
    pub capture_started_at: DateTime<Utc>,
    pub duration_ms: i32,
    pub client_chunk_id: String,
    pub created_at: DateTime<Utc>,
}

/// Check whether an idempotent retry of a given `client_chunk_id` already
/// exists for this (session, pseudo_id). If so, return the existing row.
pub async fn lookup_by_client_id(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    pseudo_id: &PseudoId,
    client_chunk_id: &str,
) -> Result<Option<ChunkRow>, AppError> {
    let row = sqlx::query_as::<_, ChunkRow>(
        "SELECT * FROM session_chunks
         WHERE session_id = $1 AND pseudo_id = $2 AND client_chunk_id = $3",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .bind(client_chunk_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row)
}

/// Insert a chunk row with a server-assigned monotonic seq per
/// `(session_id, pseudo_id)`.
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    pseudo_id: &PseudoId,
    s3_key: &str,
    size_bytes: i32,
    capture_started_at: DateTime<Utc>,
    duration_ms: i32,
    client_chunk_id: &str,
) -> Result<ChunkRow, AppError> {
    let row = sqlx::query_as::<_, ChunkRow>(
        "INSERT INTO session_chunks (
            session_id, pseudo_id, seq, s3_key, size_bytes,
            capture_started_at, duration_ms, client_chunk_id
         )
         VALUES (
            $1, $2,
            COALESCE((SELECT MAX(seq) FROM session_chunks
                      WHERE session_id = $1 AND pseudo_id = $2), -1) + 1,
            $3, $4, $5, $6, $7
         )
         RETURNING *",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .bind(s3_key)
    .bind(size_bytes)
    .bind(capture_started_at)
    .bind(duration_ms)
    .bind(client_chunk_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

pub async fn list(
    pool: &PgPool,
    session_id: Uuid,
    pseudo_id: &PseudoId,
) -> Result<Vec<ChunkRow>, AppError> {
    let rows = sqlx::query_as::<_, ChunkRow>(
        "SELECT * FROM session_chunks
         WHERE session_id = $1 AND pseudo_id = $2
         ORDER BY seq ASC",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(
    pool: &PgPool,
    session_id: Uuid,
    pseudo_id: &PseudoId,
    seq: i32,
) -> Result<ChunkRow, AppError> {
    sqlx::query_as::<_, ChunkRow>(
        "SELECT * FROM session_chunks
         WHERE session_id = $1 AND pseudo_id = $2 AND seq = $3",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .bind(seq)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        AppError::NotFound(format!(
            "chunk {seq} not found for session {session_id} / {pseudo_id}"
        ))
    })
}

/// Delete all chunk rows for a session (used by session delete cascade).
pub async fn delete_for_session(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
) -> Result<Vec<String>, AppError> {
    #[derive(sqlx::FromRow)]
    struct K {
        s3_key: String,
    }
    let rows = sqlx::query_as::<_, K>(
        "DELETE FROM session_chunks WHERE session_id = $1 RETURNING s3_key",
    )
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|r| r.s3_key).collect())
}

/// Delete chunks for a specific participant. Used by the per-participant
/// audio wipe path.
pub async fn delete_for_participant(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    pseudo_id: &PseudoId,
) -> Result<Vec<String>, AppError> {
    #[derive(sqlx::FromRow)]
    struct K {
        s3_key: String,
    }
    let rows = sqlx::query_as::<_, K>(
        "DELETE FROM session_chunks
         WHERE session_id = $1 AND pseudo_id = $2
         RETURNING s3_key",
    )
    .bind(session_id)
    .bind(pseudo_id.as_str())
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|r| r.s3_key).collect())
}
