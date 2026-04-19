//! Database connection pool initialization with online/offline separation.
//!
//! The online pool serves latency-sensitive query requests while the offline
//! pool handles throughput-oriented episode processing. Keeping them separate
//! ensures offline extraction never starves the serving path.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::AppConfig;

/// Errors that can occur during pool initialization.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// Failed to connect or configure a connection pool.
    #[error("pool connection error: {0}")]
    Connection(#[from] sqlx::Error),

    /// A required PostgreSQL extension is missing.
    #[error("required extension '{extension}' is not available")]
    MissingExtension {
        /// Name of the missing extension (e.g. `pgvector`, `pgaudit`).
        extension: String,
    },
}

/// Holds the two separated connection pools used throughout the application.
#[derive(Debug, Clone)]
pub struct DbPools {
    /// Dedicated pool for the online pipeline (query serving, low latency).
    pub online: PgPool,
    /// Separate pool for the offline pipeline (episode processing, throughput).
    pub offline: PgPool,
}

impl DbPools {
    /// Create both connection pools from application configuration.
    ///
    /// If `DATABASE_URL_ONLINE` / `DATABASE_URL_OFFLINE` are not set, both
    /// pools fall back to `DATABASE_URL` with their respective max-connection
    /// limits.
    ///
    /// After connecting, this validates that the `vector` (pgvector) and
    /// `pgaudit` extensions are installed.
    pub async fn init(config: &AppConfig) -> Result<Self, PoolError> {
        let online_url = config
            .database_url_online
            .as_deref()
            .unwrap_or(&config.database_url);

        let offline_url = config
            .database_url_offline
            .as_deref()
            .unwrap_or(&config.database_url);

        tracing::info!(
            online_max = config.online_pool_max,
            offline_max = config.offline_pool_max,
            "initializing database connection pools"
        );

        let online = PgPoolOptions::new()
            .max_connections(config.online_pool_max)
            .connect(online_url)
            .await?;

        let offline = PgPoolOptions::new()
            .max_connections(config.offline_pool_max)
            .connect(offline_url)
            .await?;

        let pools = Self { online, offline };
        pools.validate_extensions().await?;

        tracing::info!("database pools ready");
        Ok(pools)
    }

    /// Run a simple `SELECT 1` health check against the online pool.
    pub async fn health_check(&self) -> Result<(), PoolError> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.online)
            .await?;
        Ok(())
    }

    /// Run pending sqlx migrations against the offline pool.
    ///
    /// Migrations are embedded at compile time via `sqlx::migrate!()`.
    pub async fn run_migrations(&self) -> Result<(), PoolError> {
        tracing::info!("running database migrations");
        sqlx::migrate!("./migrations")
            .run(&self.offline)
            .await
            .map_err(|e| PoolError::Connection(e.into()))?;
        tracing::info!("migrations complete");
        Ok(())
    }

    /// Verify that pgvector and pgAudit extensions are available.
    async fn validate_extensions(&self) -> Result<(), PoolError> {
        let extensions: Vec<String> =
            sqlx::query_scalar("SELECT extname::text FROM pg_extension WHERE extname IN ('vector', 'pgaudit')")
                .fetch_all(&self.online)
                .await?;

        for required in &["vector", "pgaudit"] {
            if !extensions.iter().any(|e| e == required) {
                return Err(PoolError::MissingExtension {
                    extension: (*required).to_string(),
                });
            }
        }

        tracing::info!(?extensions, "required PostgreSQL extensions verified");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_extension_error_displays_name() {
        let err = PoolError::MissingExtension {
            extension: "vector".to_string(),
        };
        assert!(err.to_string().contains("vector"));
    }
}
