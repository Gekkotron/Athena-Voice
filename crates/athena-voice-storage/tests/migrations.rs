//! Tests that the initial migration applies cleanly to an in-memory `SQLite`.

use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn open_memory_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap()
}

#[tokio::test]
async fn migrations_apply_cleanly() {
    let pool = open_memory_pool().await;
    MIGRATOR.run(&pool).await.expect("migration failed");
}

#[tokio::test]
async fn migrations_create_expected_tables() {
    let pool = open_memory_pool().await;
    MIGRATOR.run(&pool).await.unwrap();

    let rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' \
         AND name NOT LIKE '_sqlx_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>(0)).collect();
    // NB: `skill_tmp` (migration 0003) is currently unused — tmp storage is
    // in-memory by design (transient across restarts) — but the migration
    // stays because it is already applied to existing databases.
    assert_eq!(
        names,
        vec![
            "errors".to_string(),
            "events".to_string(),
            "satellites".to_string(),
            "scheduled_events".to_string(),
            "sessions".to_string(),
            "skill_kv".to_string(),
            "skill_tmp".to_string(),
        ]
    );
}

#[tokio::test]
async fn migrations_are_idempotent() {
    let pool = open_memory_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
}
