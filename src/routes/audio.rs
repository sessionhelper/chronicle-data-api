use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Serialize;
use uuid::Uuid;

use crate::db::sessions as session_db;
use crate::error::AppError;
use crate::events::ApiEvent;
use crate::routes::AppState;
use crate::storage::{audio, metadata};

#[derive(Debug, Serialize)]
struct UploadResponse {
    key: String,
    seq: u32,
}

async fn upload_chunk(
    State(state): State<AppState>,
    Path((session_id, pseudo_id)): Path<(Uuid, String)>,
    body: Body,
) -> Result<Json<UploadResponse>, AppError> {
    let session = session_db::get(&state.pool, session_id).await?;
    let s3_prefix = session.s3_prefix
        .ok_or_else(|| AppError::BadRequest("session has no s3_prefix".to_string()))?;

    // Determine next sequence number by listing existing chunks
    let existing = audio::list_chunks(&state.s3_client, &state.s3_bucket, &s3_prefix, &pseudo_id).await?;
    let next_seq = existing.last().map(|c| c.seq + 1).unwrap_or(0);

    // Read the body into bytes.
    // Target chunk size is ~2MB of PCM; 3MB limit accommodates HTTP overhead.
    let bytes = axum::body::to_bytes(body, 3 * 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("read body: {e}")))?;

    let byte_len = bytes.len();
    let key = audio::upload_chunk(
        &state.s3_client,
        &state.s3_bucket,
        &s3_prefix,
        &pseudo_id,
        next_seq,
        bytes.to_vec(),
    ).await?;

    let _ = state.events.send(ApiEvent::ChunkUploaded {
        session_id,
        pseudo_id,
        seq: next_seq,
        size: byte_len,
    });

    Ok(Json(UploadResponse { key, seq: next_seq }))
}

async fn list_chunks(
    State(state): State<AppState>,
    Path((session_id, pseudo_id)): Path<(Uuid, String)>,
) -> Result<Json<Vec<audio::ChunkInfo>>, AppError> {
    let session = session_db::get(&state.pool, session_id).await?;
    let s3_prefix = session.s3_prefix
        .ok_or_else(|| AppError::BadRequest("session has no s3_prefix".to_string()))?;

    let chunks = audio::list_chunks(&state.s3_client, &state.s3_bucket, &s3_prefix, &pseudo_id).await?;
    Ok(Json(chunks))
}

async fn download_chunk(
    State(state): State<AppState>,
    Path((session_id, pseudo_id, seq)): Path<(Uuid, String, u32)>,
) -> Result<impl IntoResponse, AppError> {
    let session = session_db::get(&state.pool, session_id).await?;
    let s3_prefix = session.s3_prefix
        .ok_or_else(|| AppError::BadRequest("session has no s3_prefix".to_string()))?;

    let bytes = audio::download_chunk(&state.s3_client, &state.s3_bucket, &s3_prefix, &pseudo_id, seq).await?;

    Ok((
        [(header::CONTENT_TYPE, "audio/pcm")],
        bytes,
    ))
}

async fn combined_audio() -> Result<StatusCode, AppError> {
    // Not implemented for MVP
    Ok(StatusCode::NOT_IMPLEMENTED)
}

async fn delete_session_audio(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let session = session_db::get(&state.pool, session_id).await?;
    let s3_prefix = session.s3_prefix
        .ok_or_else(|| AppError::BadRequest("session has no s3_prefix".to_string()))?;

    let deleted = audio::delete_session_audio(&state.s3_client, &state.s3_bucket, &s3_prefix).await?;

    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

async fn write_metadata(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(input): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    let session = session_db::get(&state.pool, session_id).await?;
    let s3_prefix = session.s3_prefix
        .ok_or_else(|| AppError::BadRequest("session has no s3_prefix".to_string()))?;

    // Write both meta.json and consent.json if provided
    if let Some(meta) = input.get("meta") {
        metadata::write_metadata(&state.s3_client, &state.s3_bucket, &s3_prefix, "meta.json", meta).await?;
    }
    if let Some(consent) = input.get("consent") {
        metadata::write_metadata(&state.s3_client, &state.s3_bucket, &s3_prefix, "consent.json", consent).await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn read_metadata(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let session = session_db::get(&state.pool, session_id).await?;
    let s3_prefix = session.s3_prefix
        .ok_or_else(|| AppError::BadRequest("session has no s3_prefix".to_string()))?;

    let meta = metadata::read_metadata(&state.s3_client, &state.s3_bucket, &s3_prefix, "meta.json").await.ok();
    let consent = metadata::read_metadata(&state.s3_client, &state.s3_bucket, &s3_prefix, "consent.json").await.ok();

    Ok(Json(serde_json::json!({
        "meta": meta,
        "consent": consent,
    })))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/sessions/{id}/audio/{pseudo_id}/chunk", post(upload_chunk))
        .route("/internal/sessions/{id}/audio/{pseudo_id}/chunks", get(list_chunks))
        .route("/internal/sessions/{id}/audio/{pseudo_id}/chunk/{seq}", get(download_chunk))
        .route("/internal/sessions/{id}/audio/combined", get(combined_audio))
        .route("/internal/sessions/{id}/audio", delete(delete_session_audio))
        .route("/internal/sessions/{id}/metadata", post(write_metadata).get(read_metadata))
}
