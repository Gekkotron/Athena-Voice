use std::io::Write;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::NamedTempFile;

#[test]
fn dry_run_exits_zero_and_logs_ready() {
    let mut cfg = NamedTempFile::new().unwrap();
    writeln!(
        cfg,
        r#"
locales = ["fr", "en"]

[server]
host = "127.0.0.1"
port = 0

[storage]
database_url = "sqlite::memory:"

[mqtt]
host = "127.0.0.1"
port = 1883
client_id = "athena-voice"

[providers]
stt = "fake"
llm = "fake"
tts = "fake"
        "#
    )
    .unwrap();

    Command::cargo_bin("athena-voice")
        .unwrap()
        .args([
            "serve",
            "--dry-run",
            "--config",
            cfg.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("ready"));
}
