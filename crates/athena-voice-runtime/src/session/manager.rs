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
            },
        );
        Ok(())
    }

    #[must_use]
    pub fn get(&self, session: SessionId) -> Option<Ref<'_, SessionId, SessionState>> {
        self.map.get(&session)
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
        mgr.open(sid, sat.clone(), loc.clone(), tx, text_tx).unwrap();
        assert_eq!(mgr.len(), 1);
        let entry = mgr.get(sid).expect("present");
        assert_eq!(entry.sat, sat);
        assert_eq!(entry.locale, loc);
    }

    #[tokio::test]
    async fn open_duplicate_returns_error() {
        let mgr = SessionManager::default();
        let (sid, sat, loc, tx, text_tx) = state();
        mgr.open(sid, sat.clone(), loc.clone(), tx.clone(), text_tx.clone()).unwrap();
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
