use std::sync::Arc;

use arc_swap::ArcSwap;
use rumqttc::{AsyncClient, EventLoop, QoS};
use serde_json::json;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use athena_voice_core::event::Event;
use athena_voice_core::ids::{Locale, SatelliteId, SessionId};
use athena_voice_core::types::{AudioFrame, Transcript};
use athena_voice_providers::ProviderFactory;

use crate::intent::{IntentMatcher, RuleIndex};
use crate::mqtt::topics::{self, ParsedTopic};
use crate::pipeline::router::RouterDeps;
use crate::pipeline::{ingest, llm, router, sink, stt, tts, vad};
use crate::session::SessionManager;
use crate::wasm::dispatcher::SkillDispatcherHandle;

/// Dependencies for a running SatelliteAdapter.
pub struct SatelliteDeps {
    pub mqtt: AsyncClient,
    pub event_loop: Arc<Mutex<EventLoop>>,
    pub factory: Arc<ProviderFactory>,
    pub session_manager: Arc<SessionManager>,
    pub event_bus: broadcast::Sender<Event>,
    /// Pattern matcher — shared across all sessions.
    pub matcher: Arc<IntentMatcher>,
    /// Rule index aggregated from loaded skills. Empty until Plan 4 Task 6 ships.
    pub rules: Arc<ArcSwap<RuleIndex>>,
    /// Skill dispatcher — when present, matched intents route through the
    /// skill instead of falling back to the LLM.
    pub dispatcher: Option<SkillDispatcherHandle>,
    pub shutdown: CancellationToken,
}

/// Spawns the SatelliteAdapter: subscribes to `athena/sat/+/session/#`,
/// pumps the event loop, and on each incoming message either opens a new
/// session (spawning the full pipeline) or routes frames to an existing one.
pub fn spawn_satellite(deps: SatelliteDeps) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Subscribe to the satellite wildcard + transcript egress feedback.
        if let Err(e) = deps
            .mqtt
            .subscribe(topics::sat_wildcard(), QoS::AtLeastOnce)
            .await
        {
            warn!(error = %e, "satellite subscribe failed");
            return;
        }

        // Spawn transcript egress: subscribes to the event bus and republishes
        // TranscriptPartial/TranscriptFinal onto athena/sat/<sat>/session/<sid>/transcript.
        let egress_task = spawn_transcript_egress(
            deps.mqtt.clone(),
            deps.event_bus.clone(),
            deps.session_manager.clone(),
            deps.shutdown.clone(),
        );

        loop {
            let poll_result = {
                let mut guard = deps.event_loop.lock().await;
                tokio::select! {
                    () = deps.shutdown.cancelled() => break,
                    ev = guard.poll() => ev,
                }
            };
            match poll_result {
                Ok(rumqttc::Event::Incoming(rumqttc::Incoming::Publish(p))) => {
                    handle_publish(&deps, &p.topic, &p.payload);
                }
                Ok(_) => {}
                Err(err) => {
                    warn!(error = %err, "mqtt event loop error");
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
        deps.session_manager.cancel_all();
        egress_task.abort();
    })
}

fn handle_publish(deps: &SatelliteDeps, topic: &str, payload: &[u8]) {
    let Some(parsed) = topics::parse_satellite_topic(topic) else {
        return;
    };
    match parsed {
        ParsedTopic::Start { sat, sid } => {
            let locale = extract_locale(payload).unwrap_or_else(|| Locale::new("en").unwrap());
            open_session(deps, sat, sid, locale);
        }
        ParsedTopic::Audio { sat: _, sid } => {
            if let Some(state) = deps.session_manager.get(sid) {
                let frame = AudioFrame {
                    session: sid,
                    seq: 0,
                    pcm: bytes::Bytes::copy_from_slice(payload),
                };
                let _ = state.audio_tx.try_send(frame);
            }
        }
        ParsedTopic::Text { sat: _, sid } => {
            // Raw UTF-8 utterance injected as a final transcript, straight
            // into the session's router — lets text-only satellites skip STT.
            if let Some(state) = deps.session_manager.get(sid) {
                let text = String::from_utf8_lossy(payload).trim().to_string();
                if text.is_empty() {
                    return;
                }
                let _ = deps.event_bus.send(Event::TranscriptFinal {
                    session: sid,
                    text: text.clone(),
                });
                let _ = state.text_tx.try_send(Transcript {
                    text,
                    is_final: true,
                    confidence: None,
                });
            }
        }
        ParsedTopic::End { sat: _, sid } => {
            // Closing the session cancels the token, which drops the audio_tx
            // and causes downstream actors to flush.
            deps.session_manager.close(sid);
        }
    }
}

fn extract_locale(payload: &[u8]) -> Option<Locale> {
    let v: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let raw = v.get("locale")?.as_str()?;
    Locale::new(raw).ok()
}

fn open_session(deps: &SatelliteDeps, sat: SatelliteId, sid: SessionId, locale: Locale) {
    let (audio_tx, audio_rx) = mpsc::channel::<AudioFrame>(64);
    let (t_tx, t_rx) = mpsc::channel::<Transcript>(16);
    if deps
        .session_manager
        .open(sid, sat.clone(), locale.clone(), audio_tx, t_tx.clone())
        .is_err()
    {
        warn!(session = %sid, "session collision on start");
        return;
    }
    let cancel = deps.session_manager.get(sid).unwrap().cancel.clone();

    // event: session started
    let _ = deps.event_bus.send(Event::SessionStarted {
        session: sid,
        satellite: sat.clone(),
        locale: locale.clone(),
    });

    // Wire the actor DAG: audio → ingest → vad → stt → router → llm → tts → sink
    let (ing_tx, ing_rx) = mpsc::channel(64);
    let (vad_tx, vad_rx) = mpsc::channel(64);
    let (llm_prompt_tx, llm_prompt_rx) = mpsc::channel::<String>(4);
    let (tok_tx, tok_rx) = mpsc::channel::<String>(64);
    let (chunk_tx, chunk_rx) = mpsc::channel::<bytes::Bytes>(64);

    // audio_rx → ingest → ing_tx
    ingest::spawn_ingest(audio_rx, ing_tx, cancel.clone());
    // ing_rx → vad → vad_tx
    vad::spawn_vad(ing_rx, vad_tx, cancel.clone(), 25);
    // vad_rx → stt → t_tx
    stt::spawn_stt(
        sid,
        locale.clone(),
        deps.factory.stt(),
        vad_rx,
        t_tx,
        deps.event_bus.clone(),
        cancel.clone(),
    );
    // t_rx → router → llm_prompt_tx
    let router_deps = RouterDeps {
        llm_tx: llm_prompt_tx,
        tts_tok_tx: tok_tx.clone(),
        event_tx: deps.event_bus.clone(),
        session: sid,
        locale: locale.clone(),
        matcher: deps.matcher.clone(),
        rules: deps.rules.clone(),
        dispatcher: deps.dispatcher.clone(),
    };
    router::spawn_router(t_rx, router_deps, cancel.clone());
    // llm_prompt_rx → llm → tok_tx
    llm::spawn_llm(
        sid,
        locale.clone(),
        deps.factory.llm(),
        llm_prompt_rx,
        tok_tx,
        cancel.clone(),
    );
    // tok_rx → tts → chunk_tx
    tts::spawn_tts(
        sid,
        locale.clone(),
        deps.factory.tts(),
        tok_rx,
        chunk_tx,
        deps.event_bus.clone(),
        cancel.clone(),
    );
    // chunk_rx → sink → MQTT publish (tts/meta + tts + done)
    sink::spawn_sink(
        sid,
        sat,
        deps.mqtt.clone(),
        chunk_rx,
        deps.event_bus.clone(),
        cancel,
    );

    info!(session = %sid, "session opened");
}

enum EgressKind {
    Transcript,
    TtsText,
}

fn spawn_transcript_egress(
    mqtt: AsyncClient,
    event_tx: broadcast::Sender<Event>,
    sessions: Arc<SessionManager>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    let mut rx = event_tx.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                ev = rx.recv() => match ev {
                    Ok(event) => {
                        let (session, topic_kind, payload) = match event {
                            Event::TranscriptPartial { session, text } => (
                                session,
                                EgressKind::Transcript,
                                json!({ "is_final": false, "text": text }).to_string(),
                            ),
                            Event::TranscriptFinal { session, text } => (
                                session,
                                EgressKind::Transcript,
                                json!({ "is_final": true, "text": text }).to_string(),
                            ),
                            Event::TtsText { session, text } => (
                                session,
                                EgressKind::TtsText,
                                json!({ "text": text }).to_string(),
                            ),
                            _ => continue,
                        };
                        let sat_opt = sessions.get(session).map(|s| s.sat.clone());
                        if let Some(sat) = sat_opt {
                            let topic = match topic_kind {
                                EgressKind::Transcript => topics::session_transcript(&sat, session),
                                EgressKind::TtsText => topics::session_tts_text(&sat, session),
                            };
                            let _ = mqtt
                                .publish(topic, QoS::AtLeastOnce, false, payload)
                                .await;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {},
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    })
}
