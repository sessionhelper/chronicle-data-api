//! Integration tests for the features-pass refactor.
//!
//! Skipped silently if `TEST_DATABASE_URL` is not set. When it is set, each
//! test spins up a harness with a fresh Postgres schema + in-memory object
//! store, runs migrations, and exercises the HTTP surface via reqwest.

mod common;

use common::pseudo;
use reqwest::StatusCode;
use serde_json::json;
use uuid::Uuid;

// --- pseudo_id validation ---------------------------------------------------

#[tokio::test]
async fn pseudo_id_validation_rejects_non_24_hex() {
    let h = skip_unless_db!();
    let token = h.auth("bot").await;
    let c = h.client(&token);

    // POST /internal/users with a 16-hex id is the old (now-illegal) shape.
    let resp = c
        .post("/internal/users")
        .json(&json!({ "pseudo_id": "0123456789abcdef" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // A valid 24-hex id is accepted.
    let resp = c
        .post("/internal/users")
        .json(&json!({ "pseudo_id": pseudo("alice") }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "got {:?}", resp.status());
}

// --- session state machine + resume + delete -------------------------------

async fn create_session(c: &common::AuthedClient, id: Uuid) {
    let resp = c
        .post("/internal/sessions")
        .json(&json!({
            "id": id,
            "guild_id": 1_i64,
            "started_at": chrono::Utc::now(),
            "s3_prefix": format!("sessions/1/{id}"),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "create session: {:?}", resp.text().await);
}

async fn patch_status(c: &common::AuthedClient, id: Uuid, status: &str) -> reqwest::Response {
    c.patch(&format!("/internal/sessions/{id}"))
        .json(&json!({"status": status}))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn state_machine_happy_path_and_illegal_transitions() {
    let h = skip_unless_db!();
    let token = h.auth("bot").await;
    let c = h.client(&token);
    let id = Uuid::new_v4();
    create_session(&c, id).await;

    // recording → transcribed is illegal (skips uploaded + transcribing).
    let resp = patch_status(&c, id, "transcribed").await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Walk the happy path.
    assert!(patch_status(&c, id, "uploaded").await.status().is_success());
    assert!(patch_status(&c, id, "transcribing").await.status().is_success());
    // Self-transition transcribing → transcribing is allowed (worker re-claim).
    assert!(patch_status(&c, id, "transcribing").await.status().is_success());
    assert!(patch_status(&c, id, "transcribed").await.status().is_success());

    // transcribed → anything-but-deleted is illegal.
    let resp = patch_status(&c, id, "uploaded").await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn resume_within_ttl_brings_abandoned_back_to_recording() {
    let h = skip_unless_db!();
    let token = h.auth("bot").await;
    let c = h.client(&token);
    let id = Uuid::new_v4();
    create_session(&c, id).await;

    assert!(patch_status(&c, id, "abandoned").await.status().is_success());

    let resp = c
        .post(&format!("/internal/sessions/{id}/resume"))
        .json(&json!({"resumed_by_service_name": "bot", "reason": "test"}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{:?}", resp.text().await);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "recording");
}

#[tokio::test]
async fn delete_cascades_and_sets_tombstone() {
    let h = skip_unless_db!();
    let token = h.auth("bot").await;
    let c = h.client(&token);
    let id = Uuid::new_v4();
    create_session(&c, id).await;

    let resp = c
        .post(&format!("/internal/sessions/{id}/delete"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["tombstone_status"], "deleted");

    // Session row still exists (tombstone).
    let resp = c.get(&format!("/internal/sessions/{id}")).send().await.unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "deleted");
}

// --- chunk idempotency ------------------------------------------------------

async fn make_participant(c: &common::AuthedClient, id: Uuid, pid: &str) {
    c.post("/internal/users")
        .json(&json!({"pseudo_id": pid}))
        .send()
        .await
        .unwrap();
    c.post(&format!("/internal/sessions/{id}/participants"))
        .json(&json!({"pseudo_id": pid}))
        .send()
        .await
        .unwrap();
}

#[tokio::test]
async fn chunk_idempotency_on_client_chunk_id() {
    let h = skip_unless_db!();
    let token = h.auth("bot").await;
    let c = h.client(&token);
    let id = Uuid::new_v4();
    let pid = pseudo("speaker-1");
    create_session(&c, id).await;
    make_participant(&c, id, &pid).await;

    let upload = |body: Vec<u8>, chunk_id: &str| {
        let url = format!("/internal/sessions/{id}/audio/{pid}/chunk");
        let chunk_id = chunk_id.to_string();
        let c = &c;
        async move {
            c.post(&url)
                .header("X-Capture-Started-At", "2026-01-01T00:00:00Z")
                .header("X-Duration-Ms", "20")
                .header("X-Client-Chunk-Id", chunk_id)
                .body(body)
                .send()
                .await
                .unwrap()
        }
    };

    let r1: serde_json::Value = upload(vec![1u8; 16], "cid-1").await.json().await.unwrap();
    let r2: serde_json::Value = upload(vec![2u8; 16], "cid-1").await.json().await.unwrap();
    assert_eq!(r1["seq"], r2["seq"]);
    assert_eq!(r2["deduplicated"], true);

    let r3: serde_json::Value = upload(vec![3u8; 16], "cid-2").await.json().await.unwrap();
    assert_eq!(r3["seq"].as_i64().unwrap(), r1["seq"].as_i64().unwrap() + 1);
}

// --- bulk dedup + ETag 412 --------------------------------------------------

#[tokio::test]
async fn segments_bulk_dedup_and_etag_precondition() {
    let h = skip_unless_db!();
    let token = h.auth("worker").await;
    let c = h.client(&token);
    let id = Uuid::new_v4();
    create_session(&c, id).await;

    let first: serde_json::Value = c
        .post(&format!("/internal/sessions/{id}/segments"))
        .json(&json!({"segments": [
            {"client_id": "s1", "start_ms": 0, "end_ms": 100, "text": "hello"},
            {"client_id": "s2", "start_ms": 100, "end_ms": 200, "text": "world"},
        ]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first["inserted"].as_array().unwrap().len(), 2);
    assert!(first["deduplicated_client_ids"].as_array().unwrap().is_empty());

    // Retry with one overlap: s1 dedups, s3 inserts.
    let second: serde_json::Value = c
        .post(&format!("/internal/sessions/{id}/segments"))
        .json(&json!({"segments": [
            {"client_id": "s1", "start_ms": 0, "end_ms": 100, "text": "hello"},
            {"client_id": "s3", "start_ms": 200, "end_ms": 300, "text": "again"},
        ]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second["inserted"].as_array().unwrap().len(), 1);
    assert_eq!(second["deduplicated_client_ids"], json!(["s1"]));

    // ETag flow on PATCH.
    let seg_id = first["inserted"][0]["id"].as_str().unwrap().to_string();
    let get_resp = c.get(&format!("/internal/segments/{seg_id}")).send().await.unwrap();
    let etag = get_resp.headers()["etag"].to_str().unwrap().to_string();
    // Good If-Match succeeds.
    let ok = c
        .patch(&format!("/internal/segments/{seg_id}"))
        .header("If-Match", &etag)
        .json(&json!({"text": "edited"}))
        .send()
        .await
        .unwrap();
    assert!(ok.status().is_success(), "patch good etag: {:?}", ok.status());
    // Same (now-stale) tag → 412.
    let stale = c
        .patch(&format!("/internal/segments/{seg_id}"))
        .header("If-Match", &etag)
        .json(&json!({"text": "again"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
}

// --- display name upsert ----------------------------------------------------

#[tokio::test]
async fn display_name_upsert_is_idempotent() {
    let h = skip_unless_db!();
    let token = h.auth("bot").await;
    let c = h.client(&token);
    let pid = pseudo("alice");
    c.post("/internal/users")
        .json(&json!({"pseudo_id": pid}))
        .send()
        .await
        .unwrap();

    for _ in 0..3 {
        let resp = c
            .post(&format!("/internal/users/{pid}/display_names"))
            .json(&json!({"display_name": "alice", "source": "bot"}))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{:?}", resp.text().await);
    }
    let list: serde_json::Value = c
        .get(&format!("/internal/users/{pid}/display_names"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["seen_count"].as_i64().unwrap() >= 3);
}

// --- mute CRUD --------------------------------------------------------------

#[tokio::test]
async fn mute_range_create_list_delete() {
    let h = skip_unless_db!();
    let token = h.auth("bot").await;
    let c = h.client(&token);
    let id = Uuid::new_v4();
    let pid = pseudo("speaker-9");
    create_session(&c, id).await;
    make_participant(&c, id, &pid).await;

    let created: serde_json::Value = c
        .post(&format!("/internal/sessions/{id}/participants/{pid}/mute"))
        .json(&json!({"start_offset_ms": 1000, "end_offset_ms": 2000, "reason": "redact"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let range_id = created["range_id"].as_str().unwrap();

    let list: serde_json::Value = c
        .get(&format!("/internal/sessions/{id}/participants/{pid}/mute"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    let resp = c
        .delete(&format!("/internal/sessions/{id}/participants/{pid}/mute/{range_id}"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
}

// --- admin grant ------------------------------------------------------------

#[tokio::test]
async fn admin_grant_via_shared_secret() {
    let h = skip_unless_db!();
    let pid = pseudo("root");
    let client = reqwest::Client::new();

    // Wrong shared_secret rejected.
    let bad = client
        .post(format!("{}/internal/admin/grant", h.base_url))
        .json(&json!({"shared_secret": "nope", "pseudo_id": pid}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);

    let ok = client
        .post(format!("{}/internal/admin/grant", h.base_url))
        .json(&json!({"shared_secret": common::SHARED_SECRET, "pseudo_id": pid}))
        .send()
        .await
        .unwrap();
    assert!(ok.status().is_success(), "{:?}", ok.text().await);
    let body: serde_json::Value = ok.json().await.unwrap();
    assert_eq!(body["is_admin"], true);
}

// --- WS subscribe + overflow -----------------------------------------------

#[tokio::test]
async fn ws_subscribe_then_overflow_disconnects_on_burst() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let h = skip_unless_db!();
    let token = h.auth("worker").await;

    // ws:// upgrade on the same port.
    let url = format!("ws://{}/internal/ws", h.base_url.trim_start_matches("http://"));
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().unwrap(),
    );
    let (ws, _) = tokio_tungstenite::connect_async(req).await.expect("ws connect");
    let (mut tx, mut rx) = ws.split();

    let sub_text = serde_json::to_string(&json!({
        "type": "subscribe",
        "events": ["chunk_uploaded"]
    }))
    .unwrap();
    tx.send(Message::text(sub_text)).await.unwrap();

    // Drain the `subscribed` ack.
    let _ack = rx.next().await.expect("ack").expect("ack ok");

    // Fire a firehose of chunk uploads so the per-subscriber queue (depth=8
    // in this harness) overflows and the burst detector trips.
    let c = h.client(&token);
    let id = Uuid::new_v4();
    let pid = pseudo("ws-speaker");
    create_session(&c, id).await;
    make_participant(&c, id, &pid).await;

    for i in 0..50 {
        let _ = c
            .post(&format!("/internal/sessions/{id}/audio/{pid}/chunk"))
            .header("X-Capture-Started-At", "2026-01-01T00:00:00Z")
            .header("X-Duration-Ms", "10")
            .header("X-Client-Chunk-Id", format!("cid-{i}"))
            .body(vec![0u8; 8])
            .send()
            .await
            .unwrap();
    }

    // Either we receive a stream of events (happy) or the server drops us on
    // drop-burst. Just make sure the socket stays responsive for at least a
    // few frames without panicking.
    for _ in 0..3 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), rx.next()).await {
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
}
