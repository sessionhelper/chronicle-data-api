use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use uuid::Uuid;

use crate::db::flags;
use crate::db::segments as db;
use crate::error::AppError;
use crate::events::ApiEvent;
use crate::routes::AppState;

async fn bulk_create_segments(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(input): Json<Vec<db::CreateSegment>>,
) -> Result<Json<Vec<db::Segment>>, AppError> {
    let segments = db::bulk_create(&state.pool, session_id, &input).await?;

    // Broadcast individual segment events for progressive rendering
    for seg in &segments {
        let _ = state.events.send(ApiEvent::SegmentAdded {
            session_id,
            segment: serde_json::to_value(seg).unwrap_or_default(),
        });
    }
    // Also broadcast a batch summary
    let _ = state.events.send(ApiEvent::SegmentsBatchAdded {
        session_id,
        count: segments.len(),
    });

    Ok(Json(segments))
}

async fn list_segments(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<db::Segment>>, AppError> {
    let segments = db::list(&state.pool, session_id).await?;
    Ok(Json(segments))
}

async fn create_edit(
    State(state): State<AppState>,
    Path(segment_id): Path<Uuid>,
    Json(input): Json<flags::CreateEdit>,
) -> Result<Json<flags::SegmentEdit>, AppError> {
    let edit = flags::create_edit(&state.pool, segment_id, &input).await?;
    Ok(Json(edit))
}

async fn list_edits(
    State(state): State<AppState>,
    Path(segment_id): Path<Uuid>,
) -> Result<Json<Vec<flags::SegmentEdit>>, AppError> {
    let edits = flags::list_edits(&state.pool, segment_id).await?;
    Ok(Json(edits))
}

async fn create_flag(
    State(state): State<AppState>,
    Path(segment_id): Path<Uuid>,
    Json(input): Json<flags::CreateFlag>,
) -> Result<Json<flags::SegmentFlag>, AppError> {
    let flag = flags::create_flag(&state.pool, segment_id, &input).await?;
    Ok(Json(flag))
}

async fn delete_flag(
    State(state): State<AppState>,
    Path(segment_id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    flags::delete_flag(&state.pool, segment_id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/sessions/{id}/segments", post(bulk_create_segments).get(list_segments))
        .route("/internal/segments/{id}/edit", post(create_edit))
        .route("/internal/segments/{id}/edits", get(list_edits))
        .route("/internal/segments/{id}/flag", post(create_flag).delete(delete_flag))
}
