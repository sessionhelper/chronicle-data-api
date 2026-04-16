//! Users table — pseudo_id is the primary key; Discord IDs never land here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::ids::PseudoId;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub pseudo_id: PseudoId,
    pub is_admin: bool,
    pub data_wiped_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertUser {
    pub pseudo_id: PseudoId,
}

pub async fn upsert(pool: &PgPool, input: &UpsertUser) -> Result<User, AppError> {
    let row = sqlx::query_as::<_, User>(
        "INSERT INTO users (pseudo_id) VALUES ($1)
         ON CONFLICT (pseudo_id) DO UPDATE SET pseudo_id = EXCLUDED.pseudo_id
         RETURNING *",
    )
    .bind(input.pseudo_id.as_str())
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get(pool: &PgPool, pseudo_id: &PseudoId) -> Result<User, AppError> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE pseudo_id = $1")
        .bind(pseudo_id.as_str())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("user {pseudo_id} not found")))
}

/// List every user, newest first. Used by the admin surface.
pub async fn list_all(pool: &PgPool) -> Result<Vec<User>, AppError> {
    let rows = sqlx::query_as::<_, User>(
        "SELECT pseudo_id, is_admin, data_wiped_at, created_at
         FROM users
         ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Enriched row returned from the admin list endpoint. Joins the latest
/// display-name alias, a count of distinct sessions the user has joined,
/// the most-recent session start time they participated in, and whether
/// any of those participations have a consent_scope recorded.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AdminUserListItem {
    pub pseudo_id: PseudoId,
    pub is_admin: bool,
    pub data_wiped_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub latest_display_name: Option<String>,
    pub session_count: i64,
    pub last_active_at: Option<DateTime<Utc>>,
    pub has_consent_on_file: bool,
}

pub async fn list_all_with_details(
    pool: &PgPool,
) -> Result<Vec<AdminUserListItem>, AppError> {
    // One query via lateral joins so the output is already a flat table
    // — no N+1 per-user lookup of display names or session counts.
    let rows = sqlx::query_as::<_, AdminUserListItem>(
        "
        SELECT
            u.pseudo_id,
            u.is_admin,
            u.data_wiped_at,
            u.created_at,
            d.display_name AS latest_display_name,
            COALESCE(sp.session_count, 0) AS session_count,
            sp.last_active_at,
            COALESCE(sp.has_consent_on_file, FALSE) AS has_consent_on_file
        FROM users u
        LEFT JOIN LATERAL (
            SELECT display_name
            FROM user_display_names
            WHERE pseudo_id = u.pseudo_id
            ORDER BY last_seen_at DESC
            LIMIT 1
        ) d ON TRUE
        LEFT JOIN LATERAL (
            SELECT
                COUNT(DISTINCT sp2.session_id) AS session_count,
                MAX(s.started_at) AS last_active_at,
                bool_or(sp2.consent_scope IS NOT NULL) AS has_consent_on_file
            FROM session_participants sp2
            JOIN sessions s ON s.id = sp2.session_id
            WHERE sp2.pseudo_id = u.pseudo_id
        ) sp ON TRUE
        ORDER BY sp.last_active_at DESC NULLS LAST, u.created_at DESC
        ",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Per-session summary row for the admin detail page.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ParticipatedSession {
    pub session_id: Uuid,
    pub campaign_name: Option<String>,
    pub title: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: String,
    pub consent_scope: Option<String>,
    pub data_wiped_at: Option<DateTime<Utc>>,
}

pub async fn list_sessions_for_user(
    pool: &PgPool,
    pseudo_id: &PseudoId,
) -> Result<Vec<ParticipatedSession>, AppError> {
    // sessions table doesn't carry a `title` column today; alias NULL so
    // the struct compiles. Drop the alias if/when title lands upstream.
    let rows = sqlx::query_as::<_, ParticipatedSession>(
        "
        SELECT
            s.id              AS session_id,
            s.campaign_name,
            NULL::text        AS title,
            s.started_at,
            s.ended_at,
            s.status,
            sp.consent_scope,
            sp.data_wiped_at
        FROM session_participants sp
        JOIN sessions s ON s.id = sp.session_id
        WHERE sp.pseudo_id = $1
        ORDER BY s.started_at DESC
        ",
    )
    .bind(pseudo_id.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn set_admin(
    tx: &mut Transaction<'_, Postgres>,
    pseudo_id: &PseudoId,
    is_admin: bool,
) -> Result<User, AppError> {
    let row = sqlx::query_as::<_, User>(
        "INSERT INTO users (pseudo_id, is_admin) VALUES ($1, $2)
         ON CONFLICT (pseudo_id) DO UPDATE SET is_admin = EXCLUDED.is_admin
         RETURNING *",
    )
    .bind(pseudo_id.as_str())
    .bind(is_admin)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}
