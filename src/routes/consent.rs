//! Public consent-token routes (no auth) and internal token creation (service auth).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::middleware::ServiceSession;
use crate::db::{audit_log, chunks, consent_tokens, mute_ranges, participants, uniform};
use crate::error::AppError;
use crate::events::{AudioDeleted, Event};
use crate::ids::PseudoId;
use crate::routes::AppState;

// ---------------------------------------------------------------------------
// Internal (service-authed) token creation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    pub session_id: Uuid,
    pub participant_id: Uuid,
    pub pseudo_id: PseudoId,
}

async fn create_token(
    State(state): State<AppState>,
    Extension(svc): Extension<ServiceSession>,
    Json(input): Json<CreateTokenRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ct = consent_tokens::create(
        &state.pool,
        input.session_id,
        input.participant_id,
        &input.pseudo_id,
    )
    .await?;
    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: &svc.service_name,
            actor_pseudo: Some(input.pseudo_id.as_str()),
            session_id: Some(input.session_id),
            resource_type: "consent_token",
            resource_id: ct.token.to_string(),
            action: "created",
            detail: None,
        },
    )
    .await?;
    Ok(Json(json!({
        "token": ct.token,
        "expires_at": ct.expires_at,
    })))
}

/// Routes that go through the normal service auth middleware.
pub fn internal_routes() -> Router<AppState> {
    Router::new().route("/internal/consent-tokens", post(create_token))
}

// ---------------------------------------------------------------------------
// Public (no-auth) consent routes
// ---------------------------------------------------------------------------

async fn get_consent(
    State(state): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ctx = consent_tokens::validate(&state.pool, token)
        .await?
        .ok_or_else(|| AppError::NotFound("consent token not found or expired".to_string()))?;

    Ok(Json(json!({
        "token": ctx.token,
        "session": ctx.session,
        "participant": ctx.participant,
    })))
}

#[derive(Debug, Deserialize)]
pub struct PatchConsent {
    pub consent_scope: Option<String>,
    pub no_llm_training: Option<bool>,
    pub no_public_release: Option<bool>,
}

async fn patch_consent(
    State(state): State<AppState>,
    Path(token): Path<Uuid>,
    Json(input): Json<PatchConsent>,
) -> Result<Json<participants::Participant>, AppError> {
    let ctx = consent_tokens::validate(&state.pool, token)
        .await?
        .ok_or_else(|| AppError::NotFound("consent token not found or expired".to_string()))?;

    let participant_id = ctx.participant.id;
    let session_id = ctx.session.id;

    // Apply consent_scope if provided.
    let mut updated = ctx.participant;
    if let Some(scope) = &input.consent_scope {
        updated = participants::update_consent(
            &state.pool,
            participant_id,
            &participants::UpdateConsent {
                consent_scope: scope.clone(),
                consented_at: None,
            },
        )
        .await?;
    }

    // Apply license flags if provided.
    if input.no_llm_training.is_some() || input.no_public_release.is_some() {
        updated = participants::update_license(
            &state.pool,
            participant_id,
            &participants::UpdateLicense {
                no_llm_training: input.no_llm_training,
                no_public_release: input.no_public_release,
            },
        )
        .await?;
    }

    audit_log::append(
        &state.pool,
        &audit_log::Entry {
            actor_service: "consent_token",
            actor_pseudo: Some(updated.pseudo_id.as_str()),
            session_id: Some(session_id),
            resource_type: "participant",
            resource_id: participant_id.to_string(),
            action: "consent_patched_via_token",
            detail: Some(json!({
                "consent_scope": input.consent_scope,
                "no_llm_training": input.no_llm_training,
                "no_public_release": input.no_public_release,
            })),
        },
    )
    .await?;

    Ok(Json(updated))
}

async fn delete_audio(
    State(state): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ctx = consent_tokens::validate(&state.pool, token)
        .await?
        .ok_or_else(|| AppError::NotFound("consent token not found or expired".to_string()))?;

    let session_id = ctx.session.id;
    let pid = ctx.participant.pseudo_id.clone();

    let mut tx = state.pool.begin().await?;
    let keys = chunks::delete_for_participant(&mut tx, session_id, &pid).await?;
    let _segs = uniform::delete_for_participant_segments(&mut tx, session_id, &pid).await?;
    let _mutes = mute_ranges::delete_for_participant(&mut tx, session_id, &pid).await?;
    audit_log::append_tx(
        &mut tx,
        &audit_log::Entry {
            actor_service: "consent_token",
            actor_pseudo: Some(pid.as_str()),
            session_id: Some(session_id),
            resource_type: "participant_audio",
            resource_id: format!("{session_id}/{pid}"),
            action: "wiped_via_token",
            detail: Some(json!({
                "chunk_count": keys.len(),
            })),
        },
    )
    .await?;
    tx.commit().await?;

    participants::mark_wiped(&state.pool, session_id, &pid).await?;

    if !keys.is_empty() {
        if let Err(e) = state.store.delete_many(&keys).await {
            tracing::warn!(error = %e, "consent token audio s3 delete partial");
        }
    }

    let guild_id = sqlx::query_scalar::<_, i64>("SELECT guild_id FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&state.pool)
        .await?;
    let _ = state.events.send(Event::AudioDeleted {
        at_ts: Utc::now(),
        data: AudioDeleted {
            session_id,
            guild_id,
            pseudo_id: pid,
        },
    });

    Ok(StatusCode::NO_CONTENT)
}

/// Routes that do NOT go through auth middleware (public).
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/public/consent/{token}", get(get_consent).patch(patch_consent))
        .route("/public/consent/{token}/audio", delete(delete_audio))
}
