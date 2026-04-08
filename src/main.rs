mod auth;
mod config;
mod db;
mod error;
mod events;
mod routes;
mod storage;

use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::events::create_event_bus;
use crate::routes::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ovp_data_api=info,tower_http=info")),
        )
        .init();

    let config = Config::from_env();
    info!(bind = %config.bind_addr, "starting ovp-data-api");

    // Connect to Postgres and run migrations
    let pool = db::connect(&config.database_url).await?;
    db::run_migrations(&pool).await?;

    // Initialize S3 client
    let s3_client = storage::create_s3_client(&config).await;

    // Spawn session reaper
    auth::spawn_session_reaper(pool.clone());

    // Event bus for real-time WebSocket notifications
    let events = create_event_bus();

    // Build application state and router
    let state = AppState {
        pool,
        s3_client,
        s3_bucket: config.s3_bucket,
        shared_secret: config.shared_secret,
        events,
    };

    let app = routes::build_router(state);

    // Bind to localhost only
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    info!(addr = %config.bind_addr, "listening");

    axum::serve(listener, app).await?;

    Ok(())
}
