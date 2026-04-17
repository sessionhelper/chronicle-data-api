pub mod admin;
pub mod audit;
pub mod auth;
pub mod chunks;
pub mod consent;
pub mod health;
pub mod metadata;
pub mod metrics;
pub mod mute;
pub mod participants;
pub mod sessions;
pub mod uniform_crud;
pub mod users;
pub mod ws;

use std::sync::Arc;
use std::time::Duration;

use axum::{middleware, Router};
use sqlx::PgPool;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::TraceLayer;

use crate::auth::middleware::{require_service_auth, AuthState};
use crate::events::EventSender;
use crate::metrics::Metrics;
use crate::storage::ObjectStore;

/// Shared application state passed to every route handler.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub store: Arc<dyn ObjectStore>,
    pub shared_secret: String,
    pub events: EventSender,
    pub resume_ttl: Duration,
    pub ws_queue_depth: usize,
    pub heartbeat_reap: Duration,
    pub metrics: Metrics,
}

pub fn build_router(state: AppState) -> Router {
    // Unauthenticated set: /internal/auth, /internal/admin/grant (shared-secret
    // authenticated in-handler), /health/*, /metrics.
    let public = Router::new()
        .merge(auth::routes())
        .merge(admin::routes())
        .merge(health::routes())
        .merge(metrics::routes())
        .merge(consent::public_routes())
        .with_state(state.clone());

    let auth_state = AuthState {
        pool: state.pool.clone(),
        heartbeat_reap: state.heartbeat_reap,
    };
    let protected = Router::new()
        .merge(sessions::routes())
        .merge(users::routes())
        .merge(participants::routes())
        .merge(chunks::routes())
        .merge(metadata::routes())
        .merge(uniform_crud::routes())
        .merge(mute::routes())
        .merge(audit::routes())
        .merge(consent::internal_routes())
        .route_layer(middleware::from_fn_with_state(
            auth_state,
            require_service_auth,
        ))
        .with_state(state.clone());

    let ws_routes = Router::new()
        .merge(ws::routes())
        .with_state(state.clone());

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(ws_routes)
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
}
