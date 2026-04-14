//! Routes that project `db::uniform` onto segments / beats / scenes.
//!
//! Same handler body for every resource — we pass the `ResourceTable`
//! constant in via the handler closure rather than monomorphising three
//! slightly-different modules.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::ServiceSession;
use crate::db::{audit_log, uniform};
use crate::error::AppError;
use crate::events::{Event, ResourceRef};
use crate::ids::{ETag, PseudoId};
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct BulkRequest {
    pub segments: Option<Vec<uniform::CreateInput>>,
    pub beats: Option<Vec<uniform::CreateInput>>,
    pub scenes: Option<Vec<uniform::CreateInput>>,
}

impl BulkRequest {
    fn take(self, rt: uniform::ResourceTable) -> Vec<uniform::CreateInput> {
        match rt.resource_type {
            "segment" => self.segments.unwrap_or_default(),
            "beat" => self.beats.unwrap_or_default(),
            "scene" => self.scenes.unwrap_or_default(),
            _ => unreachable!("unknown resource_type"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub pseudo_id: Option<String>,
    pub since_ms: Option<i64>,
}

fn if_match(h: &HeaderMap) -> Option<ETag> {
    h.get("if-match")
        .and_then(|v| v.to_str().ok())
        .map(|s| ETag(s.trim_matches('"').to_string()))
}

fn row_response(row: uniform::UniformRow) -> Response {
    let etag_header = format!("\"{}\"", row.etag.as_str());
    (
        StatusCode::OK,
        [(header::ETAG, etag_header)],
        Json(row),
    )
        .into_response()
}

async fn guild_id_for_session(
    pool: &sqlx::PgPool,
    session_id: Uuid,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT guild_id FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session {session_id} not found")))
}

async fn bulk_create(
    rt: uniform::ResourceTable,
    state: AppState,
    session_id: Uuid,
    svc: ServiceSession,
    inputs: Vec<uniform::CreateInput>,
) -> Result<Json<uniform::BulkResult>, AppError> {
    let guild_id = guild_id_for_session(&state.pool, session_id).await?;
    let result = uniform::bulk_insert(&state.pool, rt, session_id, &svc.service_name, &inputs).await?;

    for row in &result.inserted {
        audit_log::append(
            &state.pool,
            &audit_log::Entry {
                actor_service: &svc.service_name,
                actor_pseudo: row.author_user_pseudo_id.as_ref().map(|p| p.as_str()),
                session_id: Some(session_id),
                resource_type: rt.resource_type,
                resource_id: row.id.to_string(),
                action: "created",
                detail: Some(json!({"client_id": row.client_id})),
            },
        )
        .await?;
        let at_ts = Utc::now();
        let data = ResourceRef {
            session_id,
            guild_id,
            id: row.id,
        };
        let event = match rt.resource_type {
            "segment" => Event::SegmentCreated { at_ts, data },
            "beat" => Event::BeatCreated { at_ts, data },
            "scene" => Event::SceneCreated { at_ts, data },
            _ => unreachable!(),
        };
        let _ = state.events.send(event);
    }
    Ok(Json(result))
}

async fn list_rows(
    rt: uniform::ResourceTable,
    state: AppState,
    session_id: Uuid,
    q: ListQuery,
) -> Result<Json<Vec<uniform::UniformRow>>, AppError> {
    let pid = match q.pseudo_id {
        Some(raw) => Some(PseudoId::new(raw)?),
        None => None,
    };
    let rows = uniform::list_by_session(&state.pool, rt, session_id, pid.as_ref(), q.since_ms).await?;
    Ok(Json(rows))
}

async fn fetch_row(
    rt: uniform::ResourceTable,
    state: AppState,
    id: Uuid,
) -> Result<Response, AppError> {
    let row = uniform::get(&state.pool, rt, id).await?;
    Ok(row_response(row))
}

async fn patch_row(
    rt: uniform::ResourceTable,
    state: AppState,
    id: Uuid,
    svc: ServiceSession,
    headers: HeaderMap,
    body: uniform::PatchInput,
) -> Result<Response, AppError> {
    let expected = if_match(&headers);
    let row = uniform::patch(&state.pool, rt, id, expected.as_ref(), &body).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: row.author_user_pseudo_id.as_ref().map(|p| p.as_str()),
            session_id: Some(row.session_id),
            resource_type: rt.resource_type,
            resource_id: id.to_string(),
            action: "patched",
            detail: None,
        },
    )
    .await?;
    let guild_id = guild_id_for_session(&state.pool, row.session_id).await?;
    let at_ts = Utc::now();
    let data = ResourceRef {
        session_id: row.session_id,
        guild_id,
        id,
    };
    let event = match rt.resource_type {
        "segment" => Event::SegmentUpdated { at_ts, data },
        "beat" => Event::BeatUpdated { at_ts, data },
        "scene" => Event::SceneUpdated { at_ts, data },
        _ => unreachable!(),
    };
    let _ = state.events.send(event);
    Ok(row_response(row))
}

async fn delete_row(
    rt: uniform::ResourceTable,
    state: AppState,
    id: Uuid,
    svc: ServiceSession,
) -> Result<StatusCode, AppError> {
    let row = uniform::delete(&state.pool, rt, id).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: None,
            session_id: Some(row.session_id),
            resource_type: rt.resource_type,
            resource_id: id.to_string(),
            action: "deleted",
            detail: None,
        },
    )
    .await?;
    let guild_id = guild_id_for_session(&state.pool, row.session_id).await?;
    let at_ts = Utc::now();
    let data = ResourceRef {
        session_id: row.session_id,
        guild_id,
        id,
    };
    let event = match rt.resource_type {
        "segment" => Event::SegmentDeleted { at_ts, data },
        "beat" => Event::BeatDeleted { at_ts, data },
        "scene" => Event::SceneDeleted { at_ts, data },
        _ => unreachable!(),
    };
    let _ = state.events.send(event);
    Ok(StatusCode::NO_CONTENT)
}

macro_rules! resource_router {
    ($rt:expr, $coll:expr, $single:expr) => {{
        Router::new()
            .route(
                concat!("/internal/sessions/{id}/", $coll),
                post({
                    let rt = $rt;
                    move |State(state): State<AppState>,
                          Path(session_id): Path<Uuid>,
                          Extension(svc): Extension<ServiceSession>,
                          Json(req): Json<BulkRequest>| async move {
                        let inputs = req.take(rt);
                        bulk_create(rt, state, session_id, svc, inputs).await
                    }
                })
                .get({
                    let rt = $rt;
                    move |State(state): State<AppState>,
                          Path(session_id): Path<Uuid>,
                          Query(q): Query<ListQuery>| async move {
                        list_rows(rt, state, session_id, q).await
                    }
                }),
            )
            .route(
                concat!("/internal/", $single, "/{id}"),
                get({
                    let rt = $rt;
                    move |State(state): State<AppState>, Path(id): Path<Uuid>| async move {
                        fetch_row(rt, state, id).await
                    }
                })
                .patch({
                    let rt = $rt;
                    move |State(state): State<AppState>,
                          Path(id): Path<Uuid>,
                          Extension(svc): Extension<ServiceSession>,
                          headers: HeaderMap,
                          Json(body): Json<uniform::PatchInput>| async move {
                        patch_row(rt, state, id, svc, headers, body).await
                    }
                })
                .delete({
                    let rt = $rt;
                    move |State(state): State<AppState>,
                          Path(id): Path<Uuid>,
                          Extension(svc): Extension<ServiceSession>| async move {
                        delete_row(rt, state, id, svc).await
                    }
                }),
            )
    }};
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .merge(resource_router!(uniform::SEGMENTS, "segments", "segments"))
        .merge(resource_router!(uniform::BEATS, "beats", "beats"))
        .merge(resource_router!(uniform::SCENES, "scenes", "scenes"))
}
