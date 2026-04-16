//! Users + display-name endpoints.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use crate::auth::middleware::ServiceSession;
use crate::db::{audit_log, display_names, users};
use crate::error::AppError;
use crate::ids::PseudoId;
use crate::routes::AppState;

async fn upsert_user(
    State(state): State<AppState>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<users::UpsertUser>,
) -> Result<Json<users::User>, AppError> {
    let pid_str = input.pseudo_id.as_str().to_string();
    let user = users::upsert(&state.pool, &input).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(&pid_str),
            session_id: None,
            resource_type: "user",
            resource_id: pid_str.clone(),
            action: "upserted",
            detail: None,
        },
    )
    .await?;
    Ok(Json(user))
}

async fn fetch_user(
    State(state): State<AppState>,
    Path(pid): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = PseudoId::new(pid)?;
    let user = users::get(&state.pool, &pid).await?;
    let aliases = display_names::list(&state.pool, &pid).await?;
    let latest = aliases.first().cloned();
    Ok(Json(json!({
        "user": user,
        "latest_display_name": latest.map(|d| d.display_name),
    })))
}

async fn post_display(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<display_names::UpsertDisplayName>,
) -> Result<Json<display_names::DisplayName>, AppError> {
    let pid = PseudoId::new(pid)?;
    // ensure user exists (FK)
    users::upsert(&state.pool, &users::UpsertUser { pseudo_id: pid.clone() }).await?;
    let row = display_names::upsert(&state.pool, &pid, &input).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(pid.as_str()),
            session_id: None,
            resource_type: "display_name",
            resource_id: format!("{}/{}", pid, row.display_name),
            action: "seen",
            detail: Some(json!({
                "source": row.source,
                "seen_count": row.seen_count,
            })),
        },
    )
    .await?;
    Ok(Json(row))
}

async fn list_display(
    State(state): State<AppState>,
    Path(pid): Path<String>,
) -> Result<Json<Vec<display_names::DisplayName>>, AppError> {
    let pid = PseudoId::new(pid)?;
    Ok(Json(display_names::list(&state.pool, &pid).await?))
}

async fn list_admin_users(
    State(state): State<AppState>,
) -> Result<Json<Vec<users::User>>, AppError> {
    Ok(Json(users::list_all(&state.pool).await?))
}

#[derive(Debug, Deserialize)]
struct PatchUserRequest {
    is_admin: bool,
}

async fn patch_user(
    State(state): State<AppState>,
    Extension(svc): Extension<ServiceSession>,
    Path(pid): Path<String>,
    Json(req): Json<PatchUserRequest>,
) -> Result<Json<users::User>, AppError> {
    let pid = PseudoId::new(pid)?;
    let mut tx = state.pool.begin().await?;
    let user = users::set_admin(&mut tx, &pid, req.is_admin).await?;
    audit_log::append_tx(
        &mut tx,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(pid.as_str()),
            session_id: None,
            resource_type: "user",
            resource_id: pid.as_str().to_string(),
            action: if req.is_admin {
                "admin_granted"
            } else {
                "admin_revoked"
            },
            detail: Some(json!({ "is_admin": req.is_admin })),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(user))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/users", post(upsert_user))
        .route("/internal/admin/users", get(list_admin_users))
        .route(
            "/internal/users/{pseudo_id}",
            get(fetch_user).patch(patch_user),
        )
        .route(
            "/internal/users/{pseudo_id}/display_names",
            post(post_display).get(list_display),
        )
}
