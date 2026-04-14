//! Session metadata blob (JSONB) — replace, shallow-merge, fetch.

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Extension, Json, Router,
};
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::ServiceSession;
use crate::db::{audit_log, metadata};
use crate::error::AppError;
use crate::ids::ETag;
use crate::routes::AppState;

fn if_match(h: &HeaderMap) -> Option<ETag> {
    h.get("if-match")
        .and_then(|v| v.to_str().ok())
        .map(|s| ETag(s.trim_matches('"').to_string()))
}

fn metadata_response(blob: metadata::MetadataBlob) -> Response {
    let etag_header = format!("\"{}\"", blob.etag.as_str());
    let body = Json(json!({
        "session_id": blob.session_id,
        "blob": blob.blob,
        "etag": blob.etag,
        "updated_at": blob.updated_at,
    }));
    (
        StatusCode::OK,
        [(header::ETAG, etag_header)],
        body,
    )
        .into_response()
}

async fn fetch(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let row = metadata::get(&state.pool, id).await?;
    Ok(metadata_response(row))
}

async fn replace(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let row = metadata::replace(&state.pool, id, body).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(id),
            resource_type: "metadata",
            resource_id: id.to_string(),
            action: "replaced",
            detail: Some(json!({"etag": row.etag})),
        },
    )
    .await?;
    Ok(metadata_response(row))
}

async fn shallow_patch(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(svc): Extension<ServiceSession>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let expected = if_match(&headers);
    let row = metadata::shallow_merge(&state.pool, id, expected.as_ref(), body).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(id),
            resource_type: "metadata",
            resource_id: id.to_string(),
            action: "merged",
            detail: Some(json!({"etag": row.etag})),
        },
    )
    .await?;
    Ok(metadata_response(row))
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/internal/sessions/{id}/metadata",
        post(replace).patch(shallow_patch).get(fetch),
    )
}
