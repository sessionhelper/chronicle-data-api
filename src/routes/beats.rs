use axum::{
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use uuid::Uuid;

use crate::db::beats as db;
use crate::error::AppError;
use crate::routes::AppState;

async fn bulk_create_beats(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(input): Json<Vec<db::CreateBeat>>,
) -> Result<Json<Vec<db::Beat>>, AppError> {
    let beats = db::bulk_create(&state.pool, session_id, &input).await?;
    Ok(Json(beats))
}

async fn list_beats(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<db::Beat>>, AppError> {
    let beats = db::list(&state.pool, session_id).await?;
    Ok(Json(beats))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/sessions/{id}/beats", post(bulk_create_beats).get(list_beats))
}
