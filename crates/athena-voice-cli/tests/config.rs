use std::io::Write;

use athena_voice_cli::config::{Config, load};
use serial_test::serial;
use tempfile::NamedTempFile;

fn write_config(contents: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
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
    // Edition 2024: set_var/remove_var are unsafe. Nextest runs each test in its own
    // process, so this global mutation cannot race with other tests.
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
