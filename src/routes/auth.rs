use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use crate::auth;
use crate::auth::middleware::hash_session_token;
use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub shared_secret: String,
    pub service_name: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub session_token: String,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatResponse {
    pub ok: bool,
}

async fn authenticate(
    State(state): State<AppState>,
    Json(req): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    if req.shared_secret != state.shared_secret {
        return Err(AppError::Unauthorized("invalid shared secret".to_string()));
    }

    let session_token = auth::generate_token();
    let token_hash = hash_session_token(&session_token);

    sqlx::query(
        "INSERT INTO service_sessions (service_name, token_hash) VALUES ($1, $2)"
    )
    .bind(&req.service_name)
    .bind(&token_hash)
    .execute(&state.pool)
    .await?;

    tracing::info!(service = %req.service_name, "service session created");

    Ok(Json(AuthResponse { session_token }))
}

async fn heartbeat(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<HeartbeatResponse>, AppError> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("missing authorization header".to_string()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized("invalid authorization format".to_string()))?;

    let token_hash = hash_session_token(token);

    let result = sqlx::query(
        "UPDATE service_sessions SET last_seen = now() WHERE token_hash = $1 AND alive = true"
    )
    .bind(&token_hash)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Unauthorized("invalid or expired session".to_string()));
    }

    Ok(Json(HeartbeatResponse { ok: true }))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/auth", post(authenticate))
        .route("/internal/heartbeat", post(heartbeat))
}
