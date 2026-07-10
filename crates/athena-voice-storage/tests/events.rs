use athena_voice_core::event::{Event, Outcome};
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};
use athena_voice_storage::{SqliteStore, Store};

async fn store() -> SqliteStore {
    SqliteStore::open("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn append_and_list_in_order() {
    let s = store().await;
    let sid = SessionId::new_v4();
    let sat = SatelliteId::new("phone-01").unwrap();
    let loc = Locale::new("fr").unwrap();
    s.record_session(sid, sat.clone(), loc.clone()).await.unwrap();

    s.append_event(&Event::SessionStarted { session: sid, satellite: sat, locale: loc })
        .await
        .unwrap();
    s.append_event(&Event::TranscriptFinal { session: sid, text: "hello".into() })
        .await
        .unwrap();
    s.append_event(&Event::SessionEnded { session: sid, outcome: Outcome::Ok })
        .await
        .unwrap();

    let rows = s.list_events_by_session(sid, 100).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].kind, "session_started");
    assert_eq!(rows[1].kind, "transcript_final");
    assert_eq!(rows[2].kind, "session_ended");
}

#[tokio::test]
async fn list_respects_limit() {
    let s = store().await;
    let sid = SessionId::new_v4();
    for _ in 0..5 {
        s.append_event(&Event::LlmFallback { session: sid }).await.unwrap();
    }
    let rows = s.list_events_by_session(sid, 3).await.unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn list_only_returns_session_events() {
    let s = store().await;
    let a = SessionId::new_v4();
    let b = SessionId::new_v4();
    s.append_event(&Event::LlmFallback { session: a }).await.unwrap();
    s.append_event(&Event::LlmFallback { session: b }).await.unwrap();
    s.append_event(&Event::LlmFallback { session: a }).await.unwrap();

    let rows = s.list_events_by_session(a, 100).await.unwrap();
    assert_eq!(rows.len(), 2);
}
