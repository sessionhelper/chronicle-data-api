//! Users table — pseudo_id is the primary key; Discord IDs never land here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};

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
