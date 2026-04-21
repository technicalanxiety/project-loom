//! Database connection pool initialization with online/offline separation.
//!
//! The online pool serves latency-sensitive query requests while the offline
//! pool handles throughput-oriented episode processing. Keeping them separate
//! ensures offline extraction never starves the serving path.
//!
//! Pool tuning parameters (acquire_timeout, idle_timeout, min_connections,
//! statement_timeout) are configurable via environment variables.

use std::time::Duration;

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

    /// Service is temporarily unavailable after retries.
    #[error("service unavailable after {attempts} attempts: {message}")]
    ServiceUnavailable {
        /// Number of retry attempts made.
        attempts: u32,
        /// Description of the failure.
        message: String,
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
    /// Pool tuning:
    /// - `acquire_timeout`: max wait time to acquire a connection from the pool
    /// - `idle_timeout`: connections idle longer than this are closed
    /// - `min_connections`: pool keeps at least this many connections open
    /// - `statement_timeout`: per-connection SQL statement timeout via `SET statement_timeout`
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

        let acquire_timeout = Duration::from_secs(config.pool_acquire_timeout_secs);
        let idle_timeout = Duration::from_secs(config.pool_idle_timeout_secs);
        let statement_timeout_ms = config.statement_timeout_secs * 1000;

        tracing::info!(
            online_max = config.online_pool_max,
            online_min = config.online_pool_min,
            offline_max = config.offline_pool_max,
            offline_min = config.offline_pool_min,
            acquire_timeout_secs = config.pool_acquire_timeout_secs,
            idle_timeout_secs = config.pool_idle_timeout_secs,
            statement_timeout_secs = config.statement_timeout_secs,
            "initializing database connection pools"
        );

        let online = PgPoolOptions::new()
            .max_connections(config.online_pool_max)
            .min_connections(config.online_pool_min)
            .acquire_timeout(acquire_timeout)
            .idle_timeout(idle_timeout)
            .after_connect(move |conn, _meta| {
                Box::pin(async move {
                    // Set per-connection statement timeout to prevent runaway queries.
                    sqlx::query(&format!("SET statement_timeout = '{statement_timeout_ms}ms'"))
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(online_url)
            .await?;

        let offline = PgPoolOptions::new()
            .max_connections(config.offline_pool_max)
            .min_connections(config.offline_pool_min)
            .acquire_timeout(acquire_timeout)
            .idle_timeout(idle_timeout)
            .after_connect(move |conn, _meta| {
                Box::pin(async move {
                    // Offline pool gets a longer statement timeout (3x) since
                    // extraction queries can be heavier.
                    let offline_timeout_ms = statement_timeout_ms * 3;
                    sqlx::query(&format!("SET statement_timeout = '{offline_timeout_ms}ms'"))
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
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

    /// Run a health check with retry logic (3 attempts, exponential backoff).
    ///
    /// Returns `Err(PoolError::ServiceUnavailable)` if all attempts fail,
    /// signaling a 503 response at the API layer.
    pub async fn health_check_with_retry(&self) -> Result<(), PoolError> {
        const MAX_RETRIES: u32 = 3;
        const BACKOFF_BASE_MS: u64 = 500;

        let mut last_err: Option<sqlx::Error> = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_millis(
                    BACKOFF_BASE_MS * 2u64.pow(attempt - 1),
                );
                tracing::warn!(
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    "retrying database health check"
                );
                tokio::time::sleep(delay).await;
            }

            match sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(&self.online)
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    tracing::warn!(
                        attempt,
                        error = %e,
                        "database health check attempt failed"
                    );
                    last_err = Some(e);
                }
            }
        }

        Err(PoolError::ServiceUnavailable {
            attempts: MAX_RETRIES,
            message: last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        })
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

// ---------------------------------------------------------------------------
// Hot tier cache
// ---------------------------------------------------------------------------

/// In-memory cache for hot tier items with TTL-based expiration.
///
/// Avoids repeated DB hits for hot tier entities/facts that are always
/// included in every compilation for a namespace. Thread-safe via
/// `RwLock`.
pub mod cache {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio::sync::RwLock;

    /// A cached value with an expiration timestamp.
    #[derive(Debug, Clone)]
    struct CacheEntry<V> {
        value: V,
        expires_at: Instant,
    }

    /// A simple in-memory cache with TTL-based expiration.
    ///
    /// Keyed by namespace string. Values are generic so the cache can
    /// store any serializable type (hot tier items, metrics, etc.).
    #[derive(Debug, Clone)]
    pub struct TtlCache<V: Clone> {
        entries: Arc<RwLock<HashMap<String, CacheEntry<V>>>>,
        ttl: Duration,
    }

    impl<V: Clone + Send + Sync + 'static> TtlCache<V> {
        /// Create a new cache with the given TTL.
        pub fn new(ttl: Duration) -> Self {
            Self {
                entries: Arc::new(RwLock::new(HashMap::new())),
                ttl,
            }
        }

        /// Get a cached value if it exists and hasn't expired.
        pub async fn get(&self, key: &str) -> Option<V> {
            let entries = self.entries.read().await;
            entries.get(key).and_then(|entry| {
                if Instant::now() < entry.expires_at {
                    Some(entry.value.clone())
                } else {
                    None
                }
            })
        }

        /// Insert or update a cached value.
        pub async fn set(&self, key: String, value: V) {
            let mut entries = self.entries.write().await;
            entries.insert(
                key,
                CacheEntry {
                    value,
                    expires_at: Instant::now() + self.ttl,
                },
            );
        }

        /// Invalidate a specific key.
        pub async fn invalidate(&self, key: &str) {
            let mut entries = self.entries.write().await;
            entries.remove(key);
        }

        /// Remove all expired entries. Call periodically to prevent unbounded growth.
        pub async fn evict_expired(&self) {
            let mut entries = self.entries.write().await;
            let now = Instant::now();
            entries.retain(|_, entry| now < entry.expires_at);
        }

        /// Return the number of entries currently in the cache (including expired).
        pub async fn len(&self) -> usize {
            self.entries.read().await.len()
        }

        /// Return true if the cache is empty.
        pub async fn is_empty(&self) -> bool {
            self.entries.read().await.is_empty()
        }

        /// Return the configured TTL.
        pub fn ttl(&self) -> Duration {
            self.ttl
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::cache::TtlCache;

    #[test]
    fn missing_extension_error_displays_name() {
        let err = PoolError::MissingExtension {
            extension: "vector".to_string(),
        };
        assert!(err.to_string().contains("vector"));
    }

    #[test]
    fn service_unavailable_error_displays_attempts() {
        let err = PoolError::ServiceUnavailable {
            attempts: 3,
            message: "connection refused".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("3 attempts"), "got: {msg}");
        assert!(msg.contains("connection refused"), "got: {msg}");
    }

    #[test]
    fn connection_error_displays_message() {
        let err = PoolError::Connection(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("pool connection error"));
    }

    // -- TtlCache tests -----------------------------------------------------

    #[tokio::test]
    async fn cache_set_and_get() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        cache.set("ns1".to_string(), "value1".to_string()).await;

        let result = cache.get("ns1").await;
        assert_eq!(result, Some("value1".to_string()));
    }

    #[tokio::test]
    async fn cache_returns_none_for_missing_key() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        assert!(cache.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn cache_expires_after_ttl() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_millis(50));
        cache.set("ns1".to_string(), "value1".to_string()).await;

        // Value should be present immediately.
        assert!(cache.get("ns1").await.is_some());

        // Wait for TTL to expire.
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Value should be expired.
        assert!(cache.get("ns1").await.is_none());
    }

    #[tokio::test]
    async fn cache_invalidate_removes_key() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        cache.set("ns1".to_string(), "value1".to_string()).await;
        assert!(cache.get("ns1").await.is_some());

        cache.invalidate("ns1").await;
        assert!(cache.get("ns1").await.is_none());
    }

    #[tokio::test]
    async fn cache_evict_expired_removes_stale_entries() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_millis(50));
        cache.set("ns1".to_string(), "value1".to_string()).await;
        cache.set("ns2".to_string(), "value2".to_string()).await;

        assert_eq!(cache.len().await, 2);

        tokio::time::sleep(Duration::from_millis(100)).await;
        cache.evict_expired().await;

        assert_eq!(cache.len().await, 0);
    }

    #[tokio::test]
    async fn cache_overwrite_updates_value() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        cache.set("ns1".to_string(), "old".to_string()).await;
        cache.set("ns1".to_string(), "new".to_string()).await;

        assert_eq!(cache.get("ns1").await, Some("new".to_string()));
    }

    #[tokio::test]
    async fn cache_len_and_is_empty() {
        let cache: TtlCache<i32> = TtlCache::new(Duration::from_secs(60));
        assert!(cache.is_empty().await);
        assert_eq!(cache.len().await, 0);

        cache.set("a".to_string(), 1).await;
        assert!(!cache.is_empty().await);
        assert_eq!(cache.len().await, 1);
    }

    #[test]
    fn cache_ttl_returns_configured_duration() {
        let cache: TtlCache<i32> = TtlCache::new(Duration::from_secs(42));
        assert_eq!(cache.ttl(), Duration::from_secs(42));
    }
}
