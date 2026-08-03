//! Assist bridge: text questions from home-automation clients
//! (topics `assist/transcription/{device}`) answered as text on
//! `assist/tts/{device}`. See docs/superpowers/specs/2026-07-31-*.md.

pub mod bridge;
pub mod topics;

pub use bridge::{AssistBridge, AssistDeps, AssistInit};
