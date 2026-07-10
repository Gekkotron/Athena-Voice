use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use athena_voice_core::event::{Event, Outcome, Stage};
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};

use crate::error::StoreError;
use crate::models::{EventRow, SatelliteRow, SessionRow};
use crate::store::Store;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub struct SqliteStore {
    pool: sqlx::SqlitePool,
}

impl SqliteStore {
    /// Opens a `SQLite` database at `url` (e.g. `"sqlite:./athena.db"` or `"sqlite::memory:"`),
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

fn decode_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> StoreError {
    StoreError::Db(sqlx::Error::Decode(Box::new(e)))
}

#[async_trait]
impl Store for SqliteStore {
    async fn record_session(
        &self,
        session: SessionId,
        satellite: SatelliteId,
        locale: Locale,
    ) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions (session, satellite, locale, started_at) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(session.to_string())
        .bind(satellite.as_str())
        .bind(locale.as_str())
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn finalize_session(
        &self,
        session: SessionId,
        outcome: Outcome,
    ) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        let outcome_str = serde_json::to_value(outcome)?
            .as_str()
            .ok_or_else(|| StoreError::NotFound("outcome serde".into()))?
            .to_string();
        let n = sqlx::query(
            "UPDATE sessions SET ended_at = ?1, outcome = ?2 WHERE session = ?3",
        )
        .bind(now)
        .bind(outcome_str)
        .bind(session.to_string())
        .execute(&self.pool)
        .await?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound(format!("session {session}")));
        }
        Ok(())
    }

    async fn get_session(&self, session: SessionId) -> Result<Option<SessionRow>, StoreError> {
        let row_opt: Option<(String, String, String, String, Option<String>, Option<String>)> =
            sqlx::query_as(
                "SELECT session, satellite, locale, started_at, ended_at, outcome \
                 FROM sessions WHERE session = ?1",
            )
            .bind(session.to_string())
            .fetch_optional(&self.pool)
            .await?;
        let Some((s, sat, loc, started, ended, outcome)) = row_opt else {
            return Ok(None);
        };
        Ok(Some(SessionRow {
            session: s.parse().map_err(decode_err)?,
            satellite: SatelliteId::new(sat).map_err(decode_err)?,
            locale: Locale::new(loc).map_err(decode_err)?,
            started_at: DateTime::parse_from_rfc3339(&started)
                .map_err(decode_err)?
                .with_timezone(&Utc),
            ended_at: ended
                .map(|s| DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&Utc)))
                .transpose()
                .map_err(decode_err)?,
            outcome: outcome
                .map(|s| serde_json::from_value::<Outcome>(serde_json::Value::String(s)))
                .transpose()?,
        }))
    }

    async fn append_event(&self, event: &Event) -> Result<(), StoreError> {
        let value = serde_json::to_value(event)?;
        let kind = value
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let session = value
            .get("session")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StoreError::NotFound("event has no `session` field".into()))?
            .to_string();
        let payload = serde_json::to_string(&value)?;
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO events (session, kind, payload, at) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(session)
        .bind(kind)
        .bind(payload)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_events_by_session(
        &self,
        session: SessionId,
        limit: u32,
    ) -> Result<Vec<EventRow>, StoreError> {
        let rows: Vec<(i64, String, String, String, String)> = sqlx::query_as(
            "SELECT id, session, kind, payload, at FROM events \
             WHERE session = ?1 ORDER BY id ASC LIMIT ?2",
        )
        .bind(session.to_string())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(id, sess, kind, payload, at)| {
                Ok(EventRow {
                    id,
                    session: sess.parse().map_err(decode_err)?,
                    kind,
                    payload: serde_json::from_str(&payload)?,
                    at: DateTime::parse_from_rfc3339(&at)
                        .map_err(decode_err)?
                        .with_timezone(&Utc),
                })
            })
            .collect()
    }

    async fn append_error(
        &self,
        session: SessionId,
        stage: Stage,
        variant: &str,
        message: &str,
    ) -> Result<(), StoreError> {
        let stage_str = serde_json::to_value(stage)?
            .as_str()
            .ok_or_else(|| StoreError::NotFound("stage serde".into()))?
            .to_string();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO errors (session, stage, variant, message, at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(session.to_string())
        .bind(stage_str)
        .bind(variant)
        .bind(message)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn skill_kv_get(&self, skill: &str, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let row: Option<(Vec<u8>,)> = sqlx::query_as(
            "SELECT value FROM skill_kv WHERE skill = ?1 AND key = ?2",
        )
        .bind(skill)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(v,)| v))
    }

    async fn skill_kv_set(&self, skill: &str, key: &str, value: &[u8]) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO skill_kv (skill, key, value) VALUES (?1, ?2, ?3) \
             ON CONFLICT(skill, key) DO UPDATE SET value = excluded.value",
        )
        .bind(skill)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn provision_satellite(
        &self,
        id: SatelliteId,
        api_key_hash: &str,
    ) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO satellites (id, api_key_hash, created_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(id) DO UPDATE SET api_key_hash = excluded.api_key_hash",
        )
        .bind(id.as_str())
        .bind(api_key_hash)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_satellite(
        &self,
        id: &SatelliteId,
    ) -> Result<Option<SatelliteRow>, StoreError> {
        let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, api_key_hash, created_at, last_seen FROM satellites WHERE id = ?1",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        let Some((raw_id, hash, created, last_seen)) = row else {
            return Ok(None);
        };
        Ok(Some(SatelliteRow {
            id: SatelliteId::new(raw_id).map_err(decode_err)?,
            api_key_hash: hash,
            created_at: DateTime::parse_from_rfc3339(&created)
                .map_err(decode_err)?
                .with_timezone(&Utc),
            last_seen: last_seen
                .map(|s| DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&Utc)))
                .transpose()
                .map_err(decode_err)?,
        }))
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
        let res = SqliteStore::open("sqlite:///nonexistent-parent-abc123xyz/db.sqlite").await;
        assert!(res.is_err(), "expected error opening under a missing parent dir");
    }
}
