//! Pipeline actors. Wired together in `Runtime::spawn` (Task 18).

pub mod ingest;
pub mod llm;
pub mod router;
pub mod stt;
pub mod vad;
