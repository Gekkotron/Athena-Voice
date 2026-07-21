//! Skill-local temporary storage.
//!
//! Keys expire after `expires_sec` seconds.
//! Storage is transient across runtime restarts.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use dashmap::DashMap;

use crate::StoreError;

pub trait TmpStore: Send + Sync {
    /// Store a key-value pair with expiration.
    fn tmp_set(&self, skill: &str, key: &str, val: Vec<u8>, expires_sec: u64) -> Result<(), StoreError>;
    
    /// Retrieve a key-value pair if not expired.
    fn tmp_get(&self, skill: &str, key: &str) -> Result<Option<Vec<u8>>, StoreError>;
    
    /// Remove expired keys.
    fn tmp_gc(&self, now_sec: u64);
}

/// In-memory TmpStore.
#[derive(Debug, Default)]
pub struct MemoryTmpStore {
    store: Arc<DashMap<(String, String), (u64, Vec<u8>)>>,
}

impl MemoryTmpStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: Arc::new(DashMap::new()),
        }
    }
}

impl TmpStore for MemoryTmpStore {
    fn tmp_set(&self, skill: &str, key: &str, val: Vec<u8>, expires_sec: u64) -> Result<(), StoreError> {
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() + expires_sec;
        self.store.insert((skill.to_string(), key.to_string()), (expires_at, val));
        Ok(())
    }

    fn tmp_get(&self, skill: &str, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let (expires_at, val) = match self.store.get(&(skill.to_string(), key.to_string())) {
            Some(entry) => entry.value().clone(),
            None => return Ok(None),
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now >= expires_at {
            self.store.remove(&(skill.to_string(), key.to_string()));
            Ok(None)
        } else {
            Ok(Some(val))
        }
    }

    fn tmp_gc(&self, now_sec: u64) {
        self.store.retain(|_, (expires_at, _)| *expires_at > now_sec);
    }
}