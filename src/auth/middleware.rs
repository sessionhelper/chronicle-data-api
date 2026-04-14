//! Bearer-token auth middleware for every `/internal/*` route except
//! `/internal/auth` and `/internal/admin/grant` (which uses the shared-secret
//! bootstrap path).

use std::time::Duration;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::PgPool;

use crate::auth::token::hash_token;
use crate::db::service_sessions;

/// State injected into handlers by this middleware.
#[derive(Clone, Debug)]
pub struct ServiceSession {
    pub service_name: String,
}

/// Middleware state bundle.
#[derive(Clone)]
pub struct AuthState {
    pub pool: PgPool,
    pub heartbeat_reap: Duration,
}

pub async fn require_service_auth(
    State(auth_state): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Result<Response, Response> {
    let token = match extract_bearer(&request) {
        Ok(t) => t,
        Err(resp) => return Err(resp),
    };

    let hash = hash_token(token);
    let name = service_sessions::lookup_alive(&auth_state.pool, &hash, auth_state.heartbeat_reap)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "auth lookup failed"})),
            )
                .into_response()
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid or expired session token"})),
            )
                .into_response()
        })?;

    request.extensions_mut().insert(ServiceSession {
        service_name: name,
    });
    Ok(next.run(request).await)
}

#[allow(clippy::result_large_err)]
fn extract_bearer(request: &Request) -> Result<&str, Response> {
    let header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "missing authorization header"})),
            )
                .into_response()
        })?;
    header.strip_prefix("Bearer ").ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid authorization format"})),
        )
            .into_response()
    })
}
