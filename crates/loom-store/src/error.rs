//! Error type shared by every store operation.

/// A convenient result alias for fallible store operations.
pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Errors that a store operation can produce.
///
/// The variants preserve the underlying source so callers can distinguish a
/// transient database fault from a schema/migration problem or a payload that
/// failed to (de)serialise. The type is `#[non_exhaustive]`: further variants
/// may be added as the persistence layer grows.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// A query or connection error surfaced by `sqlx`.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// A migration failed to apply.
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// A domain value failed to serialise to, or deserialise from, JSON.
    ///
    /// This indicates that a persisted row no longer matches the loom-core
    /// domain model — for example after an incompatible schema change.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
