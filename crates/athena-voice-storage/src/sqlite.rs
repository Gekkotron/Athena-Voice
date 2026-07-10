use std::str::FromStr;

use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::error::StoreError;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub struct SqliteStore {
    pool: sqlx::SqlitePool,
}

impl SqliteStore {
    /// Opens a SQLite database at `url` (e.g. `"sqlite:./athena.db"` or `"sqlite::memory:"`),
    /// enables WAL mode, and applies migrations.
    pub async fn open(url: &str) -> Result<Self, StoreError> {
        let opts = SqliteConnectOptions::from_str(url)
            .map_err(|e| StoreError::Db(sqlx::Error::Configuration(e.into())))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;

        MIGRATOR.run(&pool).await?;

        Ok(Self { pool })
    }

    #[must_use]
    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_memory_succeeds() {
        let store = SqliteStore::open("sqlite::memory:").await.unwrap();
        sqlx::query("SELECT COUNT(*) FROM sessions")
            .fetch_one(store.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn open_uncreatable_path_returns_error() {
        // Parent directory doesn't exist and sqlx does not auto-create parents.
        let res = SqliteStore::open("sqlite:///nonexistent-parent-abc123xyz/db.sqlite").await;
        assert!(res.is_err(), "expected error opening under a missing parent dir");
    }
}
