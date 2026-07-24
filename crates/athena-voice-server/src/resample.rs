//! Audio resampling utilities.

/// Resample audio from `from_rate` to `to_rate`.
// Staged for the audio-socket path; unused until `start_audio_socket` is implemented.
#[allow(dead_code)]
pub fn resample_audio(audio: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate {
        return audio.to_vec();
    }

    // Simple linear interpolation for demonstration.
    // In production, use a library like `rubato`.
    let ratio = to_rate as f32 / from_rate as f32;
    let new_len = (audio.len() as f32 * ratio) as usize;
    let mut resampled = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let pos = i as f32 / ratio;
        let idx = pos.floor() as usize;
        let frac = pos - idx as f32;

        let next_idx = (idx + 1).min(audio.len() - 1);
        let sample = (audio[idx] as f32 * (1.0 - frac) + audio[next_idx] as f32 * frac) as i16;
        resampled.push(sample);
    }

    resampled
}
