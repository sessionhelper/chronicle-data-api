use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::PgPool;

/// Service session info extracted by the auth middleware, available to handlers.
#[derive(Clone, Debug)]
pub struct ServiceSession {
    pub service_name: String,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    service_name: String,
}

/// Auth middleware: validates Bearer token against service_sessions table.
pub async fn require_service_auth(
    pool: axum::extract::State<PgPool>,
    mut request: Request,
    next: Next,
) -> Result<Response, Response> {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(json!({ "error": "missing authorization header" }))).into_response()
        })?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid authorization format" }))).into_response()
        })?;

    let token_hash = bcrypt_hash_for_lookup(token);

    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT service_name FROM service_sessions WHERE token_hash = $1 AND alive = true"
    )
    .bind(&token_hash)
    .fetch_optional(&*pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "auth lookup failed");
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "internal error" }))).into_response()
    })?;

    match row {
        Some(session) => {
            tracing::debug!(service = %session.service_name, "authenticated service request");
            request.extensions_mut().insert(ServiceSession {
                service_name: session.service_name,
            });
            Ok(next.run(request).await)
        }
        None => {
            Err((StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid or expired session token" }))).into_response())
        }
    }
}

/// Deterministic hash for session token storage and lookup.
/// SHA-256 hex digest — same output every time for the same input.
pub fn hash_session_token(token: &str) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(token.as_bytes());
    hex::encode(hash)
}

fn bcrypt_hash_for_lookup(token: &str) -> String {
    hash_session_token(token)
}
