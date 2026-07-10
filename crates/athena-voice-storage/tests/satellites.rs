use athena_voice_core::ids::SatelliteId;
use athena_voice_storage::{SqliteStore, Store};

#[tokio::test]
async fn provision_and_find() {
    let s = SqliteStore::open("sqlite::memory:").await.unwrap();
    let id = SatelliteId::new("phone-01").unwrap();
    s.provision_satellite(id.clone(), "hash-abc").await.unwrap();

    let row = s.find_satellite(&id).await.unwrap().unwrap();
    assert_eq!(row.id, id);
    assert_eq!(row.api_key_hash, "hash-abc");
    assert!(row.last_seen.is_none());
}

#[tokio::test]
async fn find_missing_returns_none() {
    let s = SqliteStore::open("sqlite::memory:").await.unwrap();
    let id = SatelliteId::new("phone-99").unwrap();
    assert!(s.find_satellite(&id).await.unwrap().is_none());
}

#[tokio::test]
async fn provision_upserts_key_hash() {
    let s = SqliteStore::open("sqlite::memory:").await.unwrap();
    let id = SatelliteId::new("phone-01").unwrap();
    s.provision_satellite(id.clone(), "hash-old").await.unwrap();
    s.provision_satellite(id.clone(), "hash-new").await.unwrap();
    let row = s.find_satellite(&id).await.unwrap().unwrap();
    assert_eq!(row.api_key_hash, "hash-new");
}
