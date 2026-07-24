//! Plan 9 — skill-local short-lived tmpfs.
//!
//! `host_tmp_set` / `host_tmp_get` are backed by the store's `TmpStore`
//! implementation; this test locks in the semantics skills rely on:
//! per-skill namespacing, TTL expiry, and garbage collection.

use athena_voice_storage::{SqliteStore, TmpStore};

#[tokio::test(flavor = "multi_thread")]
async fn tmp_set_get_roundtrip_namespacing_and_expiry() {
    let store = SqliteStore::open("sqlite::memory:").await.unwrap();

    store.tmp_set("timer", "k", b"v".to_vec(), 60).unwrap();
    assert_eq!(
        store.tmp_get("timer", "k").unwrap().as_deref(),
        Some(&b"v"[..])
    );

    // Other skills don't see the key (namespace isolation).
    assert!(store.tmp_get("other-skill", "k").unwrap().is_none());

    // A zero TTL is already expired on read.
    store.tmp_set("timer", "gone", b"x".to_vec(), 0).unwrap();
    assert!(store.tmp_get("timer", "gone").unwrap().is_none());

    // GC with a far-future clock drops everything still stored.
    store.tmp_gc(u64::MAX);
    assert!(store.tmp_get("timer", "k").unwrap().is_none());
}
