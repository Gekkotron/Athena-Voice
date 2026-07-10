use athena_voice_cli::logging;

#[test]
fn init_is_idempotent_error() {
    logging::init().unwrap();
    tracing::info!(target: "smoke", "hello");
    tracing::warn!(target: "smoke", value = 42, "warned");
    let err = logging::init().unwrap_err();
    assert!(matches!(err, logging::LoggingError::AlreadyInit));
}
