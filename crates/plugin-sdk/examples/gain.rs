//! Example FilmCraft WebAssembly Plugin: Simple Gain Audio DSP Effect.
//!
//! Demonstrates the clean, low-latency audio processing interface in `filmcraft-plugin-sdk`.
//! Operates in place on planar f32 channels (mono, stereo, or 5.1 surround).

use filmcraft_plugin_sdk::prelude::*;

const MANIFEST_JSON: &str = r#"{
  "id": "craft.community.gain",
  "name": "Volume Gain",
  "version": "0.1.0",
  "plugin_type": "audio_effect",
  "capabilities": {
    "memory_limit_mb": 16,
    "max_threads": 1,
    "gpu_accelerated": false
  },
  "metadata": {
    "description": "Applies transparent linear gain to all audio channels.",
    "authors": ["Community Contributor"],
    "keywords": ["gain", "volume", "audio", "dsp", "level"],
    "license": "MIT OR Apache-2.0"
  },
  "agent_hints": [
    "Use when the user asks to increase volume, lower volume, or adjust clip gain.",
    "Suitable for dialogue levelling or background music attenuation."
  ],
  "agent_examples": [
    "apply_effect(id: 'craft.community.gain', gain: 0.5) -> attenuates by half (-6 dB)",
    "apply_effect(id: 'craft.community.gain', gain: 2.0) -> doubles volume (+6 dB)"
  ],
  "parameters": [
    {
      "id": "gain",
      "name": "Linear Gain",
      "type": {
        "float": {
          "min": 0.0,
          "max": 4.0,
          "decimals": 2,
          "unit": "x"
        }
      },
      "default": 1.0,
      "animatable": true,
      "agent_hint": "Multiplier for audio samples (1.0 = unity, 0.0 = silence, 2.0 = +6 dB)"
    }
  ]
}"#;

#[derive(Default)]
pub struct GainEffect;

impl AudioEffect for GainEffect {
    fn process_block(
        &mut self,
        block: &mut AudioBlock,
        params: &serde_json::Value,
    ) -> Result<(), SdkError> {
        let gain = params
            .get("gain")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0) as f32;

        block.apply_gain(gain);
        Ok(())
    }

    fn reset(&mut self) {
        // Gain is stateless, no internal filters to clear
    }

    fn latency(&self) -> usize {
        0 // Zero-latency processing
    }
}

// Automatically exports `filmcraft_alloc`, `filmcraft_dealloc`,
// `filmcraft_get_manifest`, and `filmcraft_process_audio_block` with double panic protection.
export_audio_effect!(GainEffect, MANIFEST_JSON);

fn main() {
    println!("FilmCraft Gain Audio Plugin Example");
    println!("Compile to WASM: cargo build --target wasm32-unknown-unknown --release");
}
