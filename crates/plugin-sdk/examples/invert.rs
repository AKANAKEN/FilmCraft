//! Example FilmCraft WebAssembly Plugin: Invert Video Effect.
//!
//! Demonstrates the clean developer experience provided by `filmcraft-plugin-sdk`.
//! Can be compiled to `wasm32-unknown-unknown` as a `.wasm` binary, or run locally.

use filmcraft_plugin_sdk::prelude::*;

const MANIFEST_JSON: &str = r#"{
  "id": "craft.community.invert",
  "name": "Invert Colors",
  "version": "0.1.0",
  "plugin_type": "video_effect",
  "capabilities": {
    "memory_limit_mb": 32,
    "max_threads": 1,
    "gpu_accelerated": false
  },
  "metadata": {
    "description": "Inverts RGB channels of incoming video frames.",
    "authors": ["Community Contributor"],
    "keywords": ["invert", "negative", "vfx"],
    "license": "MIT OR Apache-2.0"
  },
  "agent_hints": [
    "Use when the user requests a 'nightmare sequence', 'film negative effect', or 'invert colors'.",
    "Ideal for horror, psychological thrillers, sci-fi glitches, or stylized transitions."
  ],
  "agent_examples": [
    "apply_effect(id: 'craft.community.invert') -> inverts all RGB channels"
  ],
  "parameters": []
}"#;

#[derive(Default)]
pub struct InvertEffect;

impl VideoEffect for InvertEffect {
    fn process_frame(
        &mut self,
        frame: &mut VideoFrame,
        _params: &serde_json::Value,
    ) -> Result<(), SdkError> {
        frame.for_each_pixel_mut(|pixel| {
            pixel.r = 255 - pixel.r;
            pixel.g = 255 - pixel.g;
            pixel.b = 255 - pixel.b;
            // Alpha remains untouched
        });
        Ok(())
    }
}

// Automatically exports `filmcraft_alloc`, `filmcraft_dealloc`,
// `filmcraft_get_manifest`, and `filmcraft_process_video_frame` with double panic protection.
export_video_effect!(InvertEffect, MANIFEST_JSON);

fn main() {
    println!("FilmCraft Invert Plugin Example");
    println!("Compile with: cargo build --target wasm32-unknown-unknown --release");
}
