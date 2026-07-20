#![deny(warnings)]
//! Persistence layer for Athena-Voice.

pub mod error;
pub mod models;
pub mod sqlite;
pub mod store;

pub use error::StoreError;
pub use sqlite::SqliteStore;

#[derive(Debug, Default)]
pub struct InMemoryStore;

pub use models::ScheduledEvent;
pub use store::Store;

use crate::models::{EventRow, SatelliteRow, SessionRow};
use async_trait::async_trait;
use athena_voice_core::event::{Event, Outcome, Stage};
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};

#[async_trait]
impl Store for InMemoryStore {
    async fn record_session(
        &self,
        _session: SessionId,
        _satellite: SatelliteId,
        _locale: Locale,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    async fn finalize_session(
        &self,
        _session: SessionId,
        _outcome: Outcome,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    async fn get_session(&self, _session: SessionId) -> Result<Option<SessionRow>, StoreError> {
        Ok(None)
    }

    async fn append_event(&self, _event: &Event) -> Result<(), StoreError> {
        Ok(())
    }

    async fn list_events_by_session(
        &self,
        _session: SessionId,
        _limit: u32,
    ) -> Result<Vec<EventRow>, StoreError> {
        Ok(vec![])
    }

    async fn append_error(
        &self,
        _session: SessionId,
        _stage: Stage,
        _variant: &str,
        _message: &str,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    async fn provision_satellite(
        &self,
        _id: SatelliteId,
        _api_key_hash: &str,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    async fn find_satellite(&self, _id: &SatelliteId) -> Result<Option<SatelliteRow>, StoreError> {
        Ok(None)
    }

    async fn schedule_event(
        &self,
        _skill: &str,
        _fires_at_ms: i64,
        _topic: &str,
        _payload: &[u8],
    ) -> Result<i64, StoreError> {
        Ok(0)
    }

    async fn pop_due_events(&self, _now_ms: i64) -> Result<Vec<ScheduledEvent>, StoreError> {
        Ok(vec![])
    }

    async fn delete_scheduled(&self, _id: i64) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn skill_kv_get(&self, _skill: &str, _key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(None)
    }

    async fn skill_kv_set(
        &self,
        _skill: &str,
        _key: &str,
        _value: &[u8],
    ) -> Result<(), StoreError> {
        Ok(())
    }

    async fn skill_kv_gc(&self, _skill: &str, _now_sec: u64) -> Result<(), StoreError> {
        Ok(())
    }
}
