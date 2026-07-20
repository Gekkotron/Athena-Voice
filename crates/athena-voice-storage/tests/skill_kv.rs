use athena_voice_storage::{SqliteStore, Store};
use std::time::{SystemTime, UNIX_EPOCH};

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
    assert_eq!(v.as_deref().map(|v| &v[8..]), Some(b"Lyon".as_slice()));
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

#[tokio::test]
async fn skill_kv_gc_deletes_expired_keys() {
    let s = store().await;
    let now_sec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    // Set keys with timestamps (mimicking what skill_kv_set now does)
    // The value column now contains the 8-byte timestamp followed by the actual data
    let mut old_payload = (now_sec - 10).to_le_bytes().to_vec();
    old_payload.extend_from_slice(b"old");
    let mut fresh_payload = (now_sec - 1).to_le_bytes().to_vec();
    fresh_payload.extend_from_slice(b"fresh");
    
    sqlx::query("INSERT INTO skill_kv (skill, key, timestamp_sec, value) VALUES (?1, ?2, ?3, ?4)")
        .bind("test")
        .bind("old")
        .bind((now_sec - 10) as i64)
        .bind(&old_payload)
        .execute(s.pool())
        .await
        .unwrap();
    
    sqlx::query("INSERT INTO skill_kv (skill, key, timestamp_sec, value) VALUES (?1, ?2, ?3, ?4)")
        .bind("test")
        .bind("fresh")
        .bind((now_sec - 1) as i64)
        .bind(&fresh_payload)
        .execute(s.pool())
        .await
        .unwrap();
    
    // GC with TTL=5: deletes "old" but keeps "fresh"
    s.skill_kv_gc("test", now_sec - 5).await.unwrap();
    
    assert!(s.skill_kv_get("test", "old").await.unwrap().is_none());
    assert_eq!(
        s.skill_kv_get("test", "fresh").await.unwrap().as_deref(),
        Some(b"fresh".as_slice())
    );
}

#[tokio::test]
async fn skill_kv_gc_ignores_other_skills() {
    let s = store().await;
    let now_sec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    // Set keys for different skills (with timestamp prefix)
    let mut payload = (now_sec - 10).to_le_bytes().to_vec();
    payload.extend_from_slice(b"data");
    
    sqlx::query("INSERT INTO skill_kv (skill, key, timestamp_sec, value) VALUES (?1, ?2, ?3, ?4)")
        .bind("test")
        .bind("key")
        .bind((now_sec - 10) as i64)
        .bind(&payload)
        .execute(s.pool())
        .await
        .unwrap();
    
    sqlx::query("INSERT INTO skill_kv (skill, key, timestamp_sec, value) VALUES (?1, ?2, ?3, ?4)")
        .bind("other")
        .bind("key")
        .bind((now_sec - 10) as i64)
        .bind(&payload)
        .execute(s.pool())
        .await
        .unwrap();
    
    // GC only affects "test"
    s.skill_kv_gc("test", now_sec - 5).await.unwrap();
    
    assert!(s.skill_kv_get("test", "key").await.unwrap().is_none());
    assert_eq!(
        s.skill_kv_get("other", "key").await.unwrap().as_deref(),
        Some(b"data".as_slice())
    );
}

#[tokio::test]
async fn state_set_prepends_timestamp_automatically() {
    let s = store().await;
    let now_sec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    // Use the storage layer directly to simulate what the host does
    s.skill_kv_set("test", "key", b"value").await.unwrap();
    
    // Verify the value was stored with the correct timestamp
    let row: (i64,) = sqlx::query_as("SELECT timestamp_sec FROM skill_kv WHERE skill = ?1 AND key = ?2")
        .bind("test")
        .bind("key")
        .fetch_one(s.pool())
        .await
        .unwrap();
    
    assert!(row.0 >= now_sec as i64 - 1); // Allow 1s tolerance
    
    // Verify the value can be retrieved correctly (skipping timestamp prefix)
    let retrieved = s.skill_kv_get("test", "key").await.unwrap();
    assert_eq!(retrieved.as_deref(), Some(b"value".as_slice()));
}
