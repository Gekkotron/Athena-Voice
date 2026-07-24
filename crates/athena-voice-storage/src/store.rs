use async_trait::async_trait;
use athena_voice_core::event::{Event, Outcome, Stage};
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};

use crate::error::StoreError;
use crate::models::{EventRow, SatelliteRow, ScheduledEvent, SessionRow};
use crate::tmp::TmpStore;

#[async_trait]
pub trait Store: Send + Sync + TmpStore {
    async fn record_session(
        &self,
        session: SessionId,
        satellite: SatelliteId,
        locale: Locale,
    ) -> Result<(), StoreError>;

    async fn finalize_session(
        &self,
        session: SessionId,
        outcome: Outcome,
    ) -> Result<(), StoreError>;

    async fn get_session(&self, session: SessionId) -> Result<Option<SessionRow>, StoreError>;

    async fn append_event(&self, event: &Event) -> Result<(), StoreError>;

    async fn list_events_by_session(
        &self,
        session: SessionId,
        limit: u32,
    ) -> Result<Vec<EventRow>, StoreError>;

    async fn append_error(
        &self,
        session: SessionId,
        stage: Stage,
        variant: &str,
        message: &str,
    ) -> Result<(), StoreError>;

    async fn provision_satellite(
        &self,
        id: SatelliteId,
        api_key_hash: &str,
    ) -> Result<(), StoreError>;

    async fn find_satellite(&self, id: &SatelliteId) -> Result<Option<SatelliteRow>, StoreError>;

    async fn schedule_event(
        &self,
        skill: &str,
        fires_at_ms: i64,
        topic: &str,
        payload: &[u8],
    ) -> Result<i64, StoreError>;

    async fn pop_due_events(&self, now_ms: i64) -> Result<Vec<ScheduledEvent>, StoreError>;

    async fn delete_scheduled(&self, id: i64) -> Result<bool, StoreError>;

    async fn skill_kv_get(&self, skill: &str, key: &str) -> Result<Option<Vec<u8>>, StoreError>;

    async fn skill_kv_set(
        &self,
        skill: &str,
        key: &str,
        value: &[u8],
    ) -> Result<(), StoreError>;

    async fn skill_kv_gc(&self, skill: &str, now_sec: u64) -> Result<(), StoreError>;
}
