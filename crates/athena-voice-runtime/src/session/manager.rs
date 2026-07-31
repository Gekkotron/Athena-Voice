use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use dashmap::mapref::one::Ref;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use athena_voice_core::ids::{Locale, SatelliteId, SessionId};
use athena_voice_core::types::{AudioFrame, Transcript};

#[derive(Debug, Error)]
#[error("session {session} already exists")]
pub struct SessionExists {
    pub session: SessionId,
}

pub struct SessionState {
    pub sat: SatelliteId,
    pub locale: Locale,
    pub cancel: CancellationToken,
    pub audio_tx: mpsc::Sender<AudioFrame>,
    /// Direct line into the session's router, used by the `.../text` ingress
    /// topic to inject a final transcript without going through STT.
    pub text_tx: mpsc::Sender<Transcript>,
    /// Unix millis of the last inbound activity (audio/text). Drives the
    /// idle reaper; refreshed via [`SessionManager::touch`].
    last_activity_ms: AtomicU64,
}

fn now_ms() -> u64 {
    #[allow(clippy::cast_possible_truncation)]
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default)]
pub struct SessionManager {
    map: DashMap<SessionId, SessionState>,
}

impl SessionManager {
    pub fn open(
        &self,
        session: SessionId,
        sat: SatelliteId,
        locale: Locale,
        audio_tx: mpsc::Sender<AudioFrame>,
        text_tx: mpsc::Sender<Transcript>,
    ) -> Result<(), SessionExists> {
        if self.map.contains_key(&session) {
            return Err(SessionExists { session });
        }
        self.map.insert(
            session,
            SessionState {
                sat,
                locale,
                cancel: CancellationToken::new(),
                audio_tx,
                text_tx,
                last_activity_ms: AtomicU64::new(now_ms()),
            },
        );
        Ok(())
    }

    #[must_use]
    pub fn get(&self, session: SessionId) -> Option<Ref<'_, SessionId, SessionState>> {
        self.map.get(&session)
    }

    /// Records inbound activity for `session` (audio or text arrived).
    pub fn touch(&self, session: SessionId) {
        if let Some(state) = self.map.get(&session) {
            state.last_activity_ms.store(now_ms(), Ordering::Relaxed);
        }
    }

    /// Closes every session idle for longer than `max_idle`, returning the
    /// reaped ids. Closing cancels the session's token, which drives the
    /// normal teardown (sink publishes `done`, `SessionEnded` fires).
    pub fn reap_idle(&self, max_idle: Duration) -> Vec<SessionId> {
        #[allow(clippy::cast_possible_truncation)]
        let cutoff = now_ms().saturating_sub(max_idle.as_millis() as u64);
        let stale: Vec<SessionId> = self
            .map
            .iter()
            .filter(|e| e.value().last_activity_ms.load(Ordering::Relaxed) < cutoff)
            .map(|e| *e.key())
            .collect();
        for sid in &stale {
            self.close(*sid);
        }
        stale
    }

    pub fn close(&self, session: SessionId) {
        if let Some((_, state)) = self.map.remove(&session) {
            state.cancel.cancel();
        }
    }

    pub fn cancel_all(&self) {
        // DashMap requires the explicit `.iter()` method; `for entry in &self.map`
        // wouldn't compile since DashMap doesn't expose IntoIterator for &Self.
        #[allow(clippy::explicit_iter_loop)]
        for entry in self.map.iter() {
            entry.value().cancel.cancel();
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::type_complexity)]
    fn state() -> (
        SessionId,
        SatelliteId,
        Locale,
        mpsc::Sender<AudioFrame>,
        mpsc::Sender<Transcript>,
    ) {
        let (tx, _rx) = mpsc::channel(1);
        let (text_tx, _text_rx) = mpsc::channel(1);
        (
            SessionId::new_v4(),
            SatelliteId::new("phone-01").unwrap(),
            Locale::new("fr").unwrap(),
            tx,
            text_tx,
        )
    }

    #[tokio::test]
    async fn open_and_get() {
        let mgr = SessionManager::default();
        let (sid, sat, loc, tx, text_tx) = state();
        mgr.open(sid, sat.clone(), loc.clone(), tx, text_tx)
            .unwrap();
        assert_eq!(mgr.len(), 1);
        let entry = mgr.get(sid).expect("present");
        assert_eq!(entry.sat, sat);
        assert_eq!(entry.locale, loc);
    }

    #[tokio::test]
    async fn open_duplicate_returns_error() {
        let mgr = SessionManager::default();
        let (sid, sat, loc, tx, text_tx) = state();
        mgr.open(sid, sat.clone(), loc.clone(), tx.clone(), text_tx.clone())
            .unwrap();
        assert!(matches!(
            mgr.open(sid, sat, loc, tx, text_tx),
            Err(SessionExists { .. })
        ));
    }

    #[tokio::test]
    async fn close_cancels_and_removes() {
        let mgr = SessionManager::default();
        let (sid, sat, loc, tx, text_tx) = state();
        mgr.open(sid, sat, loc, tx, text_tx).unwrap();
        let token = mgr.get(sid).unwrap().cancel.clone();
        mgr.close(sid);
        assert_eq!(mgr.len(), 0);
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn reaper_closes_only_idle_sessions() {
        let mgr = SessionManager::default();
        let (idle_sid, sat, loc, tx, text_tx) = state();
        mgr.open(idle_sid, sat, loc, tx, text_tx).unwrap();
        let (active_sid, sat, loc, tx, text_tx) = state();
        mgr.open(active_sid, sat, loc, tx, text_tx).unwrap();
        let idle_token = mgr.get(idle_sid).unwrap().cancel.clone();

        // Nothing is stale yet.
        assert!(mgr.reap_idle(Duration::from_millis(80)).is_empty());

        tokio::time::sleep(Duration::from_millis(120)).await;
        // Activity refreshes the deadline for one session only.
        mgr.touch(active_sid);

        let reaped = mgr.reap_idle(Duration::from_millis(80));
        assert_eq!(reaped, vec![idle_sid]);
        assert!(idle_token.is_cancelled(), "reap must cancel the session");
        assert!(mgr.get(idle_sid).is_none());
        assert!(mgr.get(active_sid).is_some(), "touched session survives");
    }

    #[tokio::test]
    async fn cancel_all_fires_every_token() {
        let mgr = SessionManager::default();
        let mut tokens = Vec::new();
        for _ in 0..3 {
            let (sid, sat, loc, tx, text_tx) = state();
            mgr.open(sid, sat, loc, tx, text_tx).unwrap();
            tokens.push(mgr.get(sid).unwrap().cancel.clone());
        }
        mgr.cancel_all();
        for t in tokens {
            assert!(t.is_cancelled());
        }
    }
}
