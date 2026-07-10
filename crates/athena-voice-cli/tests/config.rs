use std::io::Write;

use athena_voice_cli::config::{Config, load};
use serial_test::serial;
use tempfile::NamedTempFile;

const MIN_SECTIONS: &str = r#"
[mqtt]
host = "127.0.0.1"
port = 1883
client_id = "athena-voice"

[providers]
stt = "fake"
llm = "fake"
tts = "fake"
"#;

fn write_config(contents: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    f.write_all(MIN_SECTIONS.as_bytes()).unwrap();
    f
}

#[test]
#[serial(env)]
fn parses_valid_toml() {
    let f = write_config(
        r#"
locales = ["fr", "en"]

[server]
host = "127.0.0.1"
port = 9000

[storage]
database_url = "sqlite::memory:"
        "#,
    );
    let c: Config = load(f.path()).unwrap();
    assert_eq!(c.server.host, "127.0.0.1");
    assert_eq!(c.server.port, 9000);
    assert_eq!(c.storage.database_url, "sqlite::memory:");
    assert_eq!(c.locales.len(), 2);
    assert_eq!(c.mqtt.host, "127.0.0.1");
    assert_eq!(c.mqtt.port, 1883);
    assert_eq!(c.mqtt.client_id, "athena-voice");
    assert!(matches!(
        c.providers.stt,
        athena_voice_providers::StageChoice::Fake
    ));
}

#[test]
#[serial(env)]
fn env_overrides_toml() {
    let f = write_config(
        r#"
locales = ["fr"]

[server]
host = "127.0.0.1"
port = 8080

[storage]
database_url = "sqlite::memory:"
        "#,
    );
    // SAFETY: single-threaded test process; no other threads reading env.
    unsafe {
        std::env::set_var("ATHENA__SERVER__PORT", "9999");
    }
    let c: Config = load(f.path()).unwrap();
    unsafe {
        std::env::remove_var("ATHENA__SERVER__PORT");
    }
    assert_eq!(c.server.port, 9999);
}

#[test]
fn rejects_invalid_locale() {
    let f = write_config(
        r#"
locales = ["french"]

[server]
host = "0.0.0.0"
port = 8080

[storage]
database_url = "sqlite::memory:"
        "#,
    );
    assert!(load(f.path()).is_err());
}

#[test]
fn missing_file_returns_error() {
    let path = std::path::Path::new("/definitely/not/a/real/path.toml");
    assert!(load(path).is_err());
}
