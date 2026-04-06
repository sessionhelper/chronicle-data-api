use axum::{
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use uuid::Uuid;

use crate::db::scenes as db;
use crate::error::AppError;
use crate::routes::AppState;

async fn bulk_create_scenes(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(input): Json<Vec<db::CreateScene>>,
) -> Result<Json<Vec<db::Scene>>, AppError> {
    let scenes = db::bulk_create(&state.pool, session_id, &input).await?;
    Ok(Json(scenes))
}

async fn list_scenes(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<Vec<db::Scene>>, AppError> {
    let scenes = db::list(&state.pool, session_id).await?;
    Ok(Json(scenes))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/sessions/{id}/scenes", post(bulk_create_scenes).get(list_scenes))
}
