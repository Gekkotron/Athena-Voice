use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use athena_voice_core::event::Outcome;
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub session: SessionId,
    pub satellite: SatelliteId,
    pub locale: Locale,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub session: SessionId,
    pub kind: String,
    pub payload: serde_json::Value,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorRow {
    pub id: i64,
    pub session: SessionId,
    pub stage: String,
    pub variant: String,
    pub message: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatelliteRow {
    pub id: SatelliteId,
    pub api_key_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_seen: Option<DateTime<Utc>>,
}
