//! Postgres persistence (hand-written SQLx; cratestack codegen deferred — ADR-0005).
//!
//! Runtime queries only (no compile-time `query!`), so the crate builds without a database. The
//! pool is optional: absent `DATABASE_URL` the control plane runs in a degraded, in-memory mode
//! (dev) and readiness reports it.

use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Connect to `DATABASE_URL` and run migrations. Returns `None` when the URL is unset (dev) or the
/// connection/migration fails (readiness then fails closed). Never panics.
pub async fn connect_from_env() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = match PgPoolOptions::new().max_connections(5).connect(&url).await {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(%error, "failed to connect to DATABASE_URL");
            return None;
        }
    };
    if let Err(error) = sqlx::migrate!().run(&pool).await {
        tracing::error!(%error, "database migrations failed");
        return None;
    }
    tracing::info!("database connected and migrations applied");
    Some(pool)
}

/// Persist a GitHub delivery, using its `delivery_id` PRIMARY KEY for exactly-once handling.
/// Returns `true` if the delivery is new (inserted), `false` if it was already seen (duplicate).
pub async fn record_delivery(
    pool: &PgPool,
    delivery_id: &str,
    event_name: &str,
    payload: &Value,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO github_deliveries (delivery_id, event_name, payload_json) \
         VALUES ($1, $2, $3) ON CONFLICT (delivery_id) DO NOTHING",
    )
    .bind(delivery_id)
    .bind(event_name)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Liveness of the connection pool (used by readiness).
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await.map(|_| ())
}
