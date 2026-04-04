pub mod middleware;

use rand::Rng;
use tracing::{info, warn};

/// Generate a cryptographically random token string (64 hex chars).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Spawn the session reaper that marks stale sessions as dead.
pub fn spawn_session_reaper(pool: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let result = sqlx::query(
                "UPDATE service_sessions SET alive = false WHERE alive = true AND last_seen < now() - interval '90 seconds'"
            )
            .execute(&pool)
            .await;

            match result {
                Ok(res) => {
                    let count = res.rows_affected();
                    if count > 0 {
                        info!(count, "reaped stale service sessions");
                    }
                }
                Err(e) => {
                    warn!(error = %e, "session reaper query failed");
                }
            }
        }
    });
}
