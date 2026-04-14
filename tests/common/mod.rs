//! Shared harness for integration tests.
//!
//! Each test gets a freshly-migrated schema in a unique Postgres schema name,
//! an `AppState` pointing at that schema + an in-memory object store, and a
//! running axum router on a random port. When a test drops its `Harness`,
//! we asynchronously drop the schema so the database doesn't fill up.
//!
//! Tests are skipped unless `TEST_DATABASE_URL` is set.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use chronicle_data_api::events::create_bus;
use chronicle_data_api::metrics::Metrics;
use chronicle_data_api::routes::{build_router, AppState};
use chronicle_data_api::storage::MemStore;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

pub const SHARED_SECRET: &str = "test-shared-secret";

pub struct Harness {
    pub base_url: String,
    pub pool: PgPool,
    pub store: Arc<MemStore>,
    pub state: AppState,
    schema: String,
    root_pool: PgPool,
    _join: tokio::task::JoinHandle<()>,
}

impl Harness {
    /// Start a harness. Returns `None` if `TEST_DATABASE_URL` is unset — the
    /// caller should skip the test in that case.
    pub async fn start() -> Option<Self> {
        let database_url = std::env::var("TEST_DATABASE_URL").ok()?;

        // Each harness gets an isolated schema so concurrent tests don't
        // stomp on each other's rows.
        let schema = format!("test_{}", Uuid::new_v4().simple());
        let root_pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .expect("connect TEST_DATABASE_URL");
        root_pool
            .execute(format!(r#"CREATE SCHEMA "{schema}""#).as_str())
            .await
            .expect("create schema");

        // The per-harness pool pins search_path to the fresh schema. Every
        // connection in the pool runs the SET before handing itself out, so
        // both direct sql and migrations land in the right place.
        let schema_for_hook = schema.clone();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |conn, _| {
                let schema = schema_for_hook.clone();
                Box::pin(async move {
                    sqlx::query(&format!(r#"SET search_path TO "{schema}""#))
                        .execute(conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(&database_url)
            .await
            .expect("connect pool");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");

        let store = Arc::new(MemStore::new());
        let state = AppState {
            pool: pool.clone(),
            store: store.clone(),
            shared_secret: SHARED_SECRET.to_string(),
            events: create_bus(),
            resume_ttl: Duration::from_secs(60),
            ws_queue_depth: 8,
            heartbeat_reap: Duration::from_secs(90),
            metrics: Metrics::new(),
        };

        let app = build_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let join = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Some(Self {
            base_url: format!("http://{addr}"),
            pool,
            store,
            state,
            schema,
            root_pool,
            _join: join,
        })
    }

    /// Authenticate as `service_name` and return the session token.
    pub async fn auth(&self, service_name: &str) -> String {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/internal/auth", self.base_url))
            .json(&serde_json::json!({
                "shared_secret": SHARED_SECRET,
                "service_name": service_name,
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "auth failed: {:?}", resp.text().await);
        let body: serde_json::Value = resp.json().await.unwrap();
        body["session_token"].as_str().unwrap().to_string()
    }

    pub fn client(&self, token: &str) -> AuthedClient {
        AuthedClient {
            base_url: self.base_url.clone(),
            token: token.to_string(),
            http: reqwest::Client::new(),
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Fire-and-forget cleanup. The root pool stays alive until the runtime
        // tears down alongside this drop; a blocking SQL would deadlock.
        let root = self.root_pool.clone();
        let schema = self.schema.clone();
        tokio::spawn(async move {
            let _ = root
                .execute(format!(r#"DROP SCHEMA IF EXISTS "{schema}" CASCADE"#).as_str())
                .await;
        });
    }
}

pub struct AuthedClient {
    pub base_url: String,
    pub token: String,
    pub http: reqwest::Client,
}

impl AuthedClient {
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    pub fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.get(self.url(path)).bearer_auth(&self.token)
    }

    pub fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.post(self.url(path)).bearer_auth(&self.token)
    }

    pub fn patch(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.patch(self.url(path)).bearer_auth(&self.token)
    }

    pub fn delete(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.delete(self.url(path)).bearer_auth(&self.token)
    }
}

/// Skip-the-test helper: prints a message + returns early if no DB available.
#[macro_export]
macro_rules! skip_unless_db {
    () => {{
        match $crate::common::Harness::start().await {
            Some(h) => h,
            None => {
                eprintln!("TEST_DATABASE_URL not set — skipping");
                return;
            }
        }
    }};
}

/// A valid 24-hex pseudo_id derived from the given seed string.
pub fn pseudo(seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(seed.as_bytes());
    hex::encode(&digest[..12])
}
