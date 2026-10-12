//! Core contracts and interfaces for FilmCraft plugins.
//!
//! Follows the "Never crash" rule: inputs (dimensions, sample rates, buffer bounds)
//! are treated as hostile, checked up front, and all processing calls return a `Result`.

use std::collections::BTreeMap;
use crate::error::PluginError;
use crate::manifest::PluginManifest;

/// Base lifecycle contract required for all FilmCraft plugins.
pub trait FilmCraftPlugin: Send + Sync {
    /// Unique identifier (e.g., `"com.storytold.grayscale"`).
    fn id(&self) -> &str;

    /// Human-readable display name.
    fn name(&self) -> &str;

    /// Declarative plugin manifest describing parameters and capabilities.
    fn manifest(&self) -> &PluginManifest;

    /// Called once when loading the plugin into the engine session.
    fn initialize(&mut self) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called before unloading or when shutting down the session.
    fn shutdown(&mut self) -> Result<(), PluginError> {
        Ok(())
    }
}

/// Specialized trait for video effects and pixel filters.
pub trait VideoEffectPlugin: FilmCraftPlugin {
    /// Process a single uncompressed RGBA8 video frame.
    ///
    /// Buffers are guaranteed to be at least `width * height * 4` bytes.
    /// Implementations must not panic on hostile dimensions, NaNs, or unexpected parameters.
    fn process_frame(
        &self,
        in_buffer: &[u8],
        out_buffer: &mut [u8],
        width: u32,
        height: u32,
        parameters: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), PluginError>;
}

/// Specialized trait for audio sample filters and DSP effects.
pub trait AudioEffectPlugin: FilmCraftPlugin {
    /// Process interleaved or channel-separated 32-bit floating point audio samples.
    fn process_audio(
        &self,
        in_samples: &[f32],
        out_samples: &mut [f32],
        channels: u16,
        sample_rate: u32,
        parameters: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), PluginError>;
}

/// Validates uncompressed RGBA8 frame buffers against hostile dimensions and potential integer overflows.
pub fn validate_frame_bounds(
    in_buffer: &[u8],
    out_buffer: &[u8],
    width: u32,
    height: u32,
) -> Result<usize, PluginError> {
    if width == 0 || height == 0 {
        return Err(PluginError::InvalidDimensions { width, height });
    }

    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| PluginError::InvalidDimensions { width, height })?;

    let expected_bytes = (pixels as usize)
        .checked_mul(4)
        .ok_or_else(|| PluginError::InvalidDimensions { width, height })?;

    if in_buffer.len() < expected_bytes {
        return Err(PluginError::BufferSizeMismatch {
            expected: expected_bytes,
            actual: in_buffer.len(),
        });
    }

    if out_buffer.len() < expected_bytes {
        return Err(PluginError::BufferSizeMismatch {
            expected: expected_bytes,
            actual: out_buffer.len(),
        });
    }

    Ok(expected_bytes)
}
