use athena_voice_storage::{SqliteStore, Store};

async fn store() -> SqliteStore {
    SqliteStore::open("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn set_then_get() {
    let s = store().await;
    s.skill_kv_set("weather", "last_city", b"Paris")
        .await
        .unwrap();
    let v = s.skill_kv_get("weather", "last_city").await.unwrap();
    assert_eq!(v.as_deref(), Some(b"Paris".as_slice()));
}

#[tokio::test]
async fn get_missing_returns_none() {
    let s = store().await;
    let v = s.skill_kv_get("weather", "nope").await.unwrap();
    assert!(v.is_none());
}

#[tokio::test]
async fn set_upserts() {
    let s = store().await;
    s.skill_kv_set("weather", "last_city", b"Paris")
        .await
        .unwrap();
    s.skill_kv_set("weather", "last_city", b"Lyon")
        .await
        .unwrap();
    let v = s.skill_kv_get("weather", "last_city").await.unwrap();
    assert_eq!(v.as_deref(), Some(b"Lyon".as_slice()));
}

#[tokio::test]
async fn kvs_are_scoped_by_skill() {
    let s = store().await;
    s.skill_kv_set("weather", "k", b"A").await.unwrap();
    s.skill_kv_set("timer", "k", b"B").await.unwrap();
    assert_eq!(
        s.skill_kv_get("weather", "k").await.unwrap().as_deref(),
        Some(b"A".as_slice())
    );
    assert_eq!(
        s.skill_kv_get("timer", "k").await.unwrap().as_deref(),
        Some(b"B".as_slice())
    );
}
