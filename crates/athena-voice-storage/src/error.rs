use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("json (de)serialization: {0}")]
    Json(#[from] serde_json::Error),
}

impl StoreError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Db(sqlx::Error::Database(e)) if e.message().contains("SQLITE_BUSY")
        )
    }
}
