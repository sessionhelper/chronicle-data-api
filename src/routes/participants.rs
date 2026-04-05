use axum::{
    extract::{Path, State},
    routing::{patch, post},
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::db::participants as db;
use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct BatchAddParticipants {
    pub participants: Vec<db::AddParticipant>,
}

async fn add_participant(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(input): Json<db::AddParticipant>,
) -> Result<Json<db::Participant>, AppError> {
    let participant = db::add(&state.pool, session_id, &input).await?;
    Ok(Json(participant))
}

/// Batch version of add_participant. Collapses an N-call loop from the
/// bot's /record flow into a single HTTP round trip and a single SQL
/// multi-row INSERT. Returns the inserted rows in input order.
async fn add_participants_batch(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(input): Json<BatchAddParticipants>,
) -> Result<Json<Vec<db::Participant>>, AppError> {
    let rows = db::add_many(&state.pool, session_id, &input.participants).await?;
    Ok(Json(rows))
}

async fn list_participants(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<db::Participant>>, AppError> {
    let participants = db::list(&state.pool, session_id).await?;
    Ok(Json(participants))
}

async fn update_consent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<db::UpdateConsent>,
) -> Result<Json<db::Participant>, AppError> {
    let participant = db::update_consent(&state.pool, id, &input).await?;
    Ok(Json(participant))
}

async fn update_license(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<db::UpdateLicense>,
) -> Result<Json<db::Participant>, AppError> {
    let participant = db::update_license(&state.pool, id, &input).await?;
    Ok(Json(participant))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/internal/sessions/{id}/participants",
            post(add_participant).get(list_participants),
        )
        .route(
            "/internal/sessions/{id}/participants/batch",
            post(add_participants_batch),
        )
        .route("/internal/participants/{id}/consent", patch(update_consent))
        .route("/internal/participants/{id}/license", patch(update_license))
}
