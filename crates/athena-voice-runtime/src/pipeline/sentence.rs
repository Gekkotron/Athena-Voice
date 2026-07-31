//! Sentence aggregation shared by the TTS actor and the assist bridge.

/// Buffered text with no sentence boundary is flushed after this much
/// token-channel silence — LLM answers don't reliably end in punctuation
/// ("je ne sais pas"), and without this they would never be spoken.
pub const IDLE_FLUSH: std::time::Duration = std::time::Duration::from_millis(800);

fn is_sentence_boundary(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// Aggregates verbatim token fragments into sentences. Producers own their
/// spacing (LLMs stream sub-word pieces), so no separators are inserted.
/// Flushes on `.`/`!`/`?` or once the buffer reaches 100 bytes.
#[derive(Default)]
pub struct SentenceBuffer {
    buf: String,
}

impl SentenceBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `token`; returns a completed sentence when this token ends
    /// one (boundary char or length cap), trimmed.
    pub fn push(&mut self, token: &str) -> Option<String> {
        self.buf.push_str(token);
        let boundary = self
            .buf
            .trim_end()
            .chars()
            .last()
            .is_some_and(is_sentence_boundary);
        if (boundary || self.buf.len() >= 100) && !self.buf.trim().is_empty() {
            let out = self.buf.trim().to_string();
            self.buf.clear();
            return Some(out);
        }
        None
    }

    /// Drains whatever is buffered (idle flush / end of stream).
    pub fn take(&mut self) -> Option<String> {
        let out = self.buf.trim().to_string();
        self.buf.clear();
        if out.is_empty() { None } else { Some(out) }
    }

    /// Drops buffered text (barge-in: the pending response is dead).
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_returns_sentence_on_boundary() {
        let mut b = SentenceBuffer::new();
        assert_eq!(b.push("Bonjour"), None);
        assert_eq!(b.push("."), Some("Bonjour.".to_string()));
        assert!(b.is_empty());
    }

    #[test]
    fn push_flushes_on_length_cap() {
        let mut b = SentenceBuffer::new();
        let long = "a".repeat(100);
        let flushed = b.push(&long);
        assert_eq!(flushed, Some(long));
    }

    #[test]
    fn tokens_are_verbatim_no_separator_injected() {
        // LLMs stream sub-word pieces ("Le", " temps") that own their spacing.
        let mut b = SentenceBuffer::new();
        b.push("Le");
        b.push(" temps");
        assert_eq!(b.push("."), Some("Le temps.".to_string()));
    }

    #[test]
    fn take_drains_unpunctuated_remainder() {
        let mut b = SentenceBuffer::new();
        b.push("je ne sais pas");
        assert_eq!(b.take(), Some("je ne sais pas".to_string()));
        assert!(b.is_empty());
        assert_eq!(b.take(), None);
    }

    #[test]
    fn take_on_whitespace_only_is_none() {
        let mut b = SentenceBuffer::new();
        b.push("   ");
        assert_eq!(b.take(), None);
        assert!(b.is_empty());
    }

    #[test]
    fn clear_drops_buffered_text() {
        let mut b = SentenceBuffer::new();
        b.push("Bonjour");
        b.clear();
        assert!(b.is_empty());
        assert_eq!(b.push("Nouveau."), Some("Nouveau.".to_string()));
    }
}
