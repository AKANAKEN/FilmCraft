//! High-level trait definitions for video effect plugins.

use crate::error::SdkError;
use crate::frame::VideoFrame;

/// The primary trait that plugin authors implement for video filters.
pub trait VideoEffect: Send + Sync {
    /// Process a frame buffer in place.
    ///
    /// The `frame` arrives populated with the incoming image data.
    fn process_frame(&mut self, frame: &mut VideoFrame, params: &serde_json::Value) -> Result<(), SdkError>;
}

/// The primary trait that plugin authors implement for audio DSP filters.
pub trait AudioEffect: Send + Sync {
    /// Process a planar block of audio in place.
    fn process_block(
        &mut self,
        block: &mut crate::audio_block::AudioBlock,
        params: &serde_json::Value,
    ) -> Result<(), SdkError>;

    /// Clear internal memory (delay lines, biquad histories, filter tails) on seek/scrub.
    fn reset(&mut self) {}

    /// Look-ahead or STFT latency in samples for FilmCraft's delay compensation.
    fn latency(&self) -> usize {
        0
    }
}
