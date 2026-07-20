//! Integration test: 24-hour soak test for the server.

use std::path::PathBuf;
use std::time::Duration;

use athena_voice_server::Config;
use tokio::net::UnixStream;
use tokio::time::sleep;
use tracing_subscriber::EnvFilter;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_soak_24h() {
    // Initialize logging.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Setup tempdir for sockets and models.
    let temp_dir = tempfile::tempdir().unwrap();
    let model_dir = temp_dir.path().join("models");
    std::fs::create_dir_all(&model_dir).unwrap();

    // Copy fixture models to tempdir.
    // TODO: Add real fixture models.

    let config = Config {
        model_dir,
        audio_socket: temp_dir.path().join("audio.sock"),
        event_socket: temp_dir.path().join("events.sock"),
        vad_aggressiveness: 2,
        asr_model: "ggml-small-french-q5_1".to_string(),
        tts_model: "piper-fr".to_string(),
        tts_voice: "bl_lightspeed".to_string(),
        tts_sample_rate: 22050,
    };

    // Start the server.
    let runtime = athena_voice_server::Runtime::new(
        config.clone(),
        Arc::new(
            athena_voice_storage::SqliteStore::in_memory()
                .await
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    tokio::spawn(async move {
        runtime.run().await.unwrap();
    });

    // Wait for sockets to be ready.
    sleep(Duration::from_secs(1)).await;

    // Connect a fake client.
    let mut audio_stream = UnixStream::connect(&config.audio_socket).await.unwrap();
    let mut event_stream = UnixStream::connect(&config.event_socket).await.unwrap();

    // Simulate 24 hours of audio clips with "Athéna" every 5 minutes.
    for i in 0..(24 * 12) {
        // 12 chunks per hour
        // Load a real audio clip containing "Athéna".
        let clip_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/athena_clip_48khz_mono_10s.wav");
        let clip_data = std::fs::read(clip_path).expect("Failed to load audio clip");

        // Send to server.
        audio_stream.write_all(&clip_data).await.unwrap();

        // Check for transcript in event stream.
        let mut buf = [0; 1024];
        if let Ok(n) = event_stream.try_read(&mut buf) {
            let transcript = String::from_utf8_lossy(&buf[..n]);
            tracing::info!("Transcript: {}", transcript);
            assert!(transcript.contains("Athéna"), "Hotword not detected");
            assert!(transcript.contains("final": true), "Not a final transcript");
        }

        sleep(Duration::from_secs(300)).await; // 5 minutes
    }
}
