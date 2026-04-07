pub mod audio;
pub mod audit;
pub mod auth;
pub mod beats;
pub mod participants;
pub mod scenes;
pub mod segments;
pub mod sessions;
pub mod users;
pub mod ws;

use axum::{middleware, Router};
use sqlx::PgPool;

use crate::auth::middleware::require_service_auth;
use crate::events::EventSender;

/// Shared application state passed to all route handlers.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub s3_client: aws_sdk_s3::Client,
    pub s3_bucket: String,
    pub shared_secret: String,
    pub events: EventSender,
}

pub fn build_router(state: AppState) -> Router {
    // Auth routes do NOT require service auth (they issue tokens)
    let auth_routes = Router::new()
        .merge(auth::routes())
        .with_state(state.clone());

    // All other routes require service auth
    let protected_routes = Router::new()
        .merge(sessions::routes())
        .merge(users::routes())
        .merge(participants::routes())
        .merge(segments::routes())
        .merge(beats::routes())
        .merge(scenes::routes())
        .merge(audio::routes())
        .merge(audit::routes())
        .route_layer(middleware::from_fn_with_state(state.pool.clone(), require_service_auth))
        .with_state(state.clone());

    // WebSocket endpoint sits outside the HTTP auth middleware — it validates
    // the token from the `?token=` query parameter during the upgrade handshake.
    let ws_routes = Router::new()
        .merge(ws::routes())
        .with_state(state.clone());

    Router::new()
        .merge(auth_routes)
        .merge(protected_routes)
        .merge(ws_routes)
}
