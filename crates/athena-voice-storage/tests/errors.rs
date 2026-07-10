use athena_voice_core::event::Stage;
use athena_voice_core::ids::SessionId;
use athena_voice_storage::{SqliteStore, Store};

#[tokio::test]
async fn append_error_persists() {
    let s = SqliteStore::open("sqlite::memory:").await.unwrap();
    let sid = SessionId::new_v4();
    s.append_error(sid, Stage::Stt, "SttError::Timeout", "provider timed out after 5000ms")
        .await
        .unwrap();

    let row: (String, String, String) = sqlx::query_as(
        "SELECT stage, variant, message FROM errors WHERE session = ?1",
    )
    .bind(sid.to_string())
    .fetch_one(s.pool())
    .await
    .unwrap();

    assert_eq!(row.0, "stt");
    assert_eq!(row.1, "SttError::Timeout");
    assert_eq!(row.2, "provider timed out after 5000ms");
}
