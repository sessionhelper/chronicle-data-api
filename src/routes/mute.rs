//! Mute ranges + per-participant hard-delete.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
    Extension, Json, Router,
};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::ServiceSession;
use crate::db::{audit_log, chunks, mute_ranges, participants, uniform};
use crate::error::AppError;
use crate::events::{AudioDeleted, Event, MuteRangeRef};
use crate::ids::PseudoId;
use crate::routes::AppState;

async fn list_mutes(
    State(state): State<AppState>,
    Path((session_id, pid_raw)): Path<(Uuid, String)>,
) -> Result<Json<Vec<mute_ranges::MuteRange>>, AppError> {
    let pid = PseudoId::new(pid_raw)?;
    Ok(Json(mute_ranges::list(&state.pool, session_id, &pid).await?))
}

async fn create_mute(
    State(state): State<AppState>,
    Path((session_id, pid_raw)): Path<(Uuid, String)>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<mute_ranges::CreateMuteRange>,
) -> Result<Json<mute_ranges::MuteRange>, AppError> {
    let pid = PseudoId::new(pid_raw)?;
    let row = mute_ranges::create(&state.pool, session_id, &pid, &input).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(pid.as_str()),
            session_id: Some(session_id),
            resource_type: "mute_range",
            resource_id: row.id.to_string(),
            action: "created",
            detail: Some(json!({
                "start_offset_ms": row.start_offset_ms,
                "end_offset_ms": row.end_offset_ms,
            })),
        },
    )
    .await?;
    let guild_id = guild_id_for_session(&state.pool, session_id).await?;
    let _ = state.events.send(Event::MuteRangeCreated {
        at_ts: Utc::now(),
        data: MuteRangeRef {
            session_id,
            guild_id,
            pseudo_id: pid,
            range_id: row.id,
        },
    });
    Ok(Json(row))
}

async fn delete_mute(
    State(state): State<AppState>,
    Path((session_id, pid_raw, range_id)): Path<(Uuid, String, Uuid)>,
    Extension(svc): Extension<ServiceSession>,
) -> Result<StatusCode, AppError> {
    let pid = PseudoId::new(pid_raw)?;
    let row = mute_ranges::delete_one(&state.pool, session_id, &pid, range_id).await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(pid.as_str()),
            session_id: Some(session_id),
            resource_type: "mute_range",
            resource_id: range_id.to_string(),
            action: "deleted",
            detail: None,
        },
    )
    .await?;
    let guild_id = guild_id_for_session(&state.pool, session_id).await?;
    let _ = state.events.send(Event::MuteRangeDeleted {
        at_ts: Utc::now(),
        data: MuteRangeRef {
            session_id,
            guild_id,
            pseudo_id: pid,
            range_id: row.id,
        },
    });
    Ok(StatusCode::NO_CONTENT)
}

/// Hard-delete a participant's audio: chunks + pipeline outputs for that
/// participant, scoped to the session. Participant row is preserved with
/// `data_wiped_at`.
async fn wipe_participant_audio(
    State(state): State<AppState>,
    Path((session_id, pid_raw)): Path<(Uuid, String)>,
    Extension(svc): Extension<ServiceSession>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pid = PseudoId::new(pid_raw)?;
    let mut tx = state.pool.begin().await?;
    let keys = chunks::delete_for_participant(&mut tx, session_id, &pid).await?;
    let segs = uniform::delete_for_participant_segments(&mut tx, session_id, &pid).await?;
    let mutes = mute_ranges::delete_for_participant(&mut tx, session_id, &pid).await?;
    audit_log::append_tx(
        &mut tx,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(pid.as_str()),
            session_id: Some(session_id),
            resource_type: "participant_audio",
            resource_id: format!("{session_id}/{pid}"),
            action: "wiped",
            detail: Some(json!({
                "chunk_count": keys.len(),
                "segment_count": segs,
                "mute_count": mutes,
            })),
        },
    )
    .await?;
    tx.commit().await?;

    // mark participant wiped (outside tx is fine; it's just a flag)
    participants::mark_wiped(&state.pool, session_id, &pid).await?;

    if !keys.is_empty() {
        if let Err(e) = state.store.delete_many(&keys).await {
            tracing::warn!(error = %e, "participant audio s3 delete partial");
        }
    }
    let guild_id = guild_id_for_session(&state.pool, session_id).await?;
    let _ = state.events.send(Event::AudioDeleted {
        at_ts: Utc::now(),
        data: AudioDeleted {
            session_id,
            guild_id,
            pseudo_id: pid,
        },
    });
    Ok(Json(json!({
        "ok": true,
        "chunk_count": keys.len(),
        "segment_count": segs,
        "mute_count": mutes,
    })))
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

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/internal/sessions/{id}/participants/{pid}/mute",
            get(list_mutes).post(create_mute),
        )
        .route(
            "/internal/sessions/{id}/participants/{pid}/mute/{range_id}",
            delete(delete_mute),
        )
        .route(
            "/internal/sessions/{id}/participants/{pid}/audio",
            delete(wipe_participant_audio),
        )
}
