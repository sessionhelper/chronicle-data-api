//! Consent token CRUD — public (no-auth) consent management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::participants::Participant;
use crate::db::sessions::Session;
use crate::error::AppError;
use crate::ids::PseudoId;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConsentToken {
    pub token: Uuid,
    pub session_id: Uuid,
    pub participant_id: Uuid,
    pub pseudo_id: PseudoId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Token joined with its session and participant rows.
#[derive(Debug, Clone, Serialize)]
pub struct ConsentTokenWithContext {
    pub token: ConsentToken,
    pub session: Session,
    pub participant: Participant,
}

pub async fn create(
    pool: &PgPool,
    session_id: Uuid,
    participant_id: Uuid,
    pseudo_id: &PseudoId,
) -> Result<ConsentToken, AppError> {
    let row = sqlx::query_as::<_, ConsentToken>(
        "INSERT INTO consent_tokens (session_id, participant_id, pseudo_id)
         VALUES ($1, $2, $3)
         RETURNING *",
    )
    .bind(session_id)
    .bind(participant_id)
    .bind(pseudo_id.as_str())
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Validate a token: return it with session + participant context only if it
/// exists, is not expired, and is not revoked.
pub async fn validate(
    pool: &PgPool,
    token: Uuid,
) -> Result<Option<ConsentTokenWithContext>, AppError> {
    let ct = sqlx::query_as::<_, ConsentToken>(
        "SELECT * FROM consent_tokens
         WHERE token = $1
           AND revoked_at IS NULL
           AND expires_at > NOW()",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;

    let ct = match ct {
        Some(ct) => ct,
        None => return Ok(None),
    };

    let session = sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions WHERE id = $1",
    )
    .bind(ct.session_id)
    .fetch_one(pool)
    .await?;

    let participant = sqlx::query_as::<_, Participant>(
        "SELECT * FROM session_participants WHERE id = $1",
    )
    .bind(ct.participant_id)
    .fetch_one(pool)
    .await?;

    Ok(Some(ConsentTokenWithContext {
        token: ct,
        session,
        participant,
    }))
}

/// Revoke a token by setting `revoked_at`.
pub async fn revoke(pool: &PgPool, token: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE consent_tokens SET revoked_at = NOW() WHERE token = $1",
    )
    .bind(token)
    .execute(pool)
    .await?;
    Ok(())
}
