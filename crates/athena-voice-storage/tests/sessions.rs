use athena_voice_core::event::Outcome;
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};
use athena_voice_storage::{SqliteStore, Store};

async fn store() -> SqliteStore {
    SqliteStore::open("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn record_and_get_session() {
    let s = store().await;
    let sid = SessionId::new_v4();
    let sat = SatelliteId::new("phone-01").unwrap();
    let loc = Locale::new("fr").unwrap();

    s.record_session(sid, sat.clone(), loc.clone()).await.unwrap();

    let row = s.get_session(sid).await.unwrap().expect("session row");
    assert_eq!(row.session, sid);
    assert_eq!(row.satellite, sat);
    assert_eq!(row.locale, loc);
    assert!(row.ended_at.is_none());
    assert!(row.outcome.is_none());
}

#[tokio::test]
async fn finalize_updates_outcome_and_ended_at() {
    let s = store().await;
    let sid = SessionId::new_v4();
    s.record_session(sid, SatelliteId::new("phone-01").unwrap(), Locale::new("en").unwrap())
        .await
        .unwrap();

    s.finalize_session(sid, Outcome::Ok).await.unwrap();

    let row = s.get_session(sid).await.unwrap().unwrap();
    assert!(row.ended_at.is_some());
    assert_eq!(row.outcome, Some(Outcome::Ok));
}

#[tokio::test]
async fn get_session_missing_returns_none() {
    let s = store().await;
    let sid = SessionId::new_v4();
    assert!(s.get_session(sid).await.unwrap().is_none());
}
