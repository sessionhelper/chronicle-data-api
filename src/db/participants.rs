use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Participant {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Option<Uuid>,
    pub consent_scope: Option<String>,
    pub consented_at: Option<DateTime<Utc>>,
    pub withdrawn_at: Option<DateTime<Utc>>,
    pub mid_session_join: bool,
    pub no_llm_training: bool,
    pub no_public_release: bool,
}

#[derive(Debug, Deserialize)]
pub struct AddParticipant {
    pub user_id: Option<Uuid>,
    pub mid_session_join: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateConsent {
    pub consent_scope: Option<String>,
    pub consented_at: Option<DateTime<Utc>>,
    pub withdrawn_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLicense {
    pub no_llm_training: Option<bool>,
    pub no_public_release: Option<bool>,
}

pub async fn add(pool: &PgPool, session_id: Uuid, input: &AddParticipant) -> Result<Participant, AppError> {
    let row = sqlx::query_as::<_, Participant>(
        "INSERT INTO session_participants (session_id, user_id, mid_session_join)
         VALUES ($1, $2, COALESCE($3, false))
         RETURNING *"
    )
    .bind(session_id)
    .bind(input.user_id)
    .bind(input.mid_session_join)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Insert N participants into a single session in one round trip. Returns
/// the inserted rows in the same order as the input. Used by the bot's
/// /record flow to replace an N-call add_participant loop with a single
/// batch INSERT, cutting the setup latency proportionally to the party
/// size.
pub async fn add_many(
    pool: &PgPool,
    session_id: Uuid,
    inputs: &[AddParticipant],
) -> Result<Vec<Participant>, AppError> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }

    // Build the VALUES clause with explicit $N placeholders. Postgres has
    // a hard cap around 65535 bind parameters; with 3 params per row that
    // caps us at ~21k participants per call, which is plenty given a
    // real-world upper bound of maybe a few dozen per session.
    let mut query = String::from(
        "INSERT INTO session_participants (session_id, user_id, mid_session_join) VALUES ",
    );
    let mut first = true;
    for i in 0..inputs.len() {
        if !first {
            query.push_str(", ");
        }
        first = false;
        let base = i * 3;
        query.push_str(&format!(
            "(${}, ${}, COALESCE(${}, false))",
            base + 1,
            base + 2,
            base + 3
        ));
    }
    query.push_str(" RETURNING *");

    let mut q = sqlx::query_as::<_, Participant>(&query);
    for input in inputs {
        q = q.bind(session_id).bind(input.user_id).bind(input.mid_session_join);
    }

    Ok(q.fetch_all(pool).await?)
}

/// A participant row enriched with the joined user's `pseudo_id`. This is
/// the shape returned by the list endpoint because every downstream caller
/// (the worker, most notably) needs the pseudo_id to address per-speaker
/// audio chunks in S3 — and the Data API is the only place that can cheaply
/// perform the join. Older callers that deserialize into a struct without
/// `user_pseudo_id` keep working since serde ignores unknown fields.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ParticipantWithUser {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Option<Uuid>,
    pub user_pseudo_id: Option<String>,
    pub consent_scope: Option<String>,
    pub consented_at: Option<DateTime<Utc>>,
    pub withdrawn_at: Option<DateTime<Utc>>,
    pub mid_session_join: bool,
    pub no_llm_training: bool,
    pub no_public_release: bool,
}

pub async fn list(pool: &PgPool, session_id: Uuid) -> Result<Vec<ParticipantWithUser>, AppError> {
    // LEFT JOIN so participants without a linked user row still appear —
    // the worker filters those out by the None pseudo_id, but we don't
    // want to silently drop rows from the listing.
    let rows = sqlx::query_as::<_, ParticipantWithUser>(
        "SELECT sp.id, sp.session_id, sp.user_id,
                u.pseudo_id AS user_pseudo_id,
                sp.consent_scope, sp.consented_at, sp.withdrawn_at,
                sp.mid_session_join, sp.no_llm_training, sp.no_public_release
         FROM session_participants sp
         LEFT JOIN users u ON u.id = sp.user_id
         WHERE sp.session_id = $1"
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Update a participant's consent state and append a matching row to
/// `consent_audit_log` in the same transaction. This is the canonical
/// place every consent decision is audited — callers never need to write
/// to the audit log explicitly.
pub async fn update_consent(pool: &PgPool, id: Uuid, input: &UpdateConsent) -> Result<Participant, AppError> {
    let mut tx = pool.begin().await?;

    // Read the existing row so the audit entry can capture previous_scope.
    let existing = sqlx::query_as::<_, Participant>(
        "SELECT * FROM session_participants WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("participant {id} not found")))?;

    let updated = sqlx::query_as::<_, Participant>(
        "UPDATE session_participants SET
            consent_scope = COALESCE($2, consent_scope),
            consented_at = COALESCE($3, consented_at),
            withdrawn_at = COALESCE($4, withdrawn_at)
         WHERE id = $1
         RETURNING *"
    )
    .bind(id)
    .bind(&input.consent_scope)
    .bind(input.consented_at)
    .bind(input.withdrawn_at)
    .fetch_one(&mut *tx)
    .await?;

    // Derive the audit action. Withdrawal wins over scope changes because
    // a withdrawal patch typically also nulls/updates consent_scope.
    //   - grant:  first-time null → <scope>
    //   - update: <scope_a> → <scope_b> (e.g., full → decline)
    //   - withdraw: withdrawn_at flipped from null
    let action = if input.withdrawn_at.is_some() && existing.withdrawn_at.is_none() {
        Some("withdraw")
    } else if existing.consent_scope != updated.consent_scope {
        if existing.consent_scope.is_none() {
            Some("grant")
        } else {
            Some("update")
        }
    } else {
        None
    };

    if let Some(action) = action {
        sqlx::query(
            "INSERT INTO consent_audit_log (user_id, session_id, action, previous_scope, new_scope)
             VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(updated.user_id)
        .bind(updated.session_id)
        .bind(action)
        .bind(&existing.consent_scope)
        .bind(&updated.consent_scope)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(updated)
}

pub async fn update_license(pool: &PgPool, id: Uuid, input: &UpdateLicense) -> Result<Participant, AppError> {
    let row = sqlx::query_as::<_, Participant>(
        "UPDATE session_participants SET
            no_llm_training = COALESCE($2, no_llm_training),
            no_public_release = COALESCE($3, no_public_release)
         WHERE id = $1
         RETURNING *"
    )
    .bind(id)
    .bind(input.no_llm_training)
    .bind(input.no_public_release)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("participant {id} not found")))?;

    Ok(row)
}
