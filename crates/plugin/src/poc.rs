//! Reference Proof-of-Concept (PoC) plugin: Grayscale Video Effect.
//!
//! Demonstrates the stateless, safe execution of a video effect conforming to `VideoEffectPlugin`.

use std::collections::BTreeMap;

use crate::error::PluginError;
use crate::manifest::{
    PluginCapabilities, PluginMetadata, PluginManifest, PluginParamDef, PluginParamKind, PluginType,
};
use crate::traits::{validate_frame_bounds, FilmCraftPlugin, VideoEffectPlugin};

/// Proof-of-concept grayscale filter.
#[derive(Clone, Debug)]
pub struct GrayscaleEffect {
    manifest: PluginManifest,
}

impl GrayscaleEffect {
    /// Creates a new instance of the reference grayscale effect.
    pub fn new() -> Self {
        let manifest = PluginManifest {
            id: "craft.builtin.poc_grayscale".to_string(),
            name: "Grayscale (PoC)".to_string(),
            version: "0.1.0".to_string(),
            plugin_type: PluginType::VideoEffect,
            capabilities: PluginCapabilities {
                memory_limit_mb: 64,
                max_threads: 1,
                gpu_accelerated: false,
            },
            metadata: PluginMetadata {
                description: "Converts video frames to black and white with controllable intensity.".to_string(),
                authors: vec!["FilmCraft Team".to_string()],
                keywords: vec!["grayscale".to_string(), "monochrome".to_string(), "black and white".to_string()],
                license: Some("MIT OR Apache-2.0".to_string()),
                repository: None,
            },
            agent_hints: vec![
                "Use when the user requests a black and white, monochrome, or noir look.".to_string(),
                "Suitable for flashbacks, dramatic pauses, or vintage cinema styles.".to_string(),
            ],
            agent_examples: vec![
                "apply_effect(id: 'craft.builtin.poc_grayscale', amount: 1.0) -> full black and white".to_string(),
                "apply_effect(id: 'craft.builtin.poc_grayscale', amount: 0.5) -> subtle desaturation".to_string(),
            ],
            parameters: vec![PluginParamDef {
                id: "amount".to_string(),
                name: "Amount".to_string(),
                kind: PluginParamKind::Float {
                    min: 0.0,
                    max: 1.0,
                    decimals: 2,
                    unit: "%".to_string(),
                },
                default: serde_json::json!(1.0),
                group: None,
                animatable: true,
                agent_hint: Some("Intensity of the monochrome conversion (0.0 = original colour, 1.0 = fully grayscale)".to_string()),
            }],
        };

        Self { manifest }
    }
}

impl Default for GrayscaleEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl FilmCraftPlugin for GrayscaleEffect {
    fn id(&self) -> &str {
        &self.manifest.id
    }

    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
}

impl VideoEffectPlugin for GrayscaleEffect {
    fn process_frame(
        &self,
        in_buffer: &[u8],
        out_buffer: &mut [u8],
        width: u32,
        height: u32,
        parameters: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), PluginError> {
        let expected_bytes = validate_frame_bounds(in_buffer, out_buffer, width, height)?;

        // Extract amount parameter (0.0 to 1.0, default 1.0).
        let amount = if let Some(val) = parameters.get("amount") {
            val.as_f64().unwrap_or(1.0).clamp(0.0, 1.0) as f32
        } else {
            1.0f32
        };

        // Rec. 709 luma coefficients
        let rw = 0.2126f32;
        let gw = 0.7152f32;
        let bw = 0.0722f32;

        let num_pixels = expected_bytes / 4;
        for i in 0..num_pixels {
            let offset = i * 4;
            let r = in_buffer[offset] as f32;
            let g = in_buffer[offset + 1] as f32;
            let b = in_buffer[offset + 2] as f32;
            let a = in_buffer[offset + 3];

            let luma = rw * r + gw * g + bw * b;

            // Linear blend between original and monochrome based on `amount`
            let final_r = r + (luma - r) * amount;
            let final_g = g + (luma - g) * amount;
            let final_b = b + (luma - b) * amount;

            out_buffer[offset] = final_r.clamp(0.0, 255.0) as u8;
            out_buffer[offset + 1] = final_g.clamp(0.0, 255.0) as u8;
            out_buffer[offset + 2] = final_b.clamp(0.0, 255.0) as u8;
            out_buffer[offset + 3] = a;
        }

        Ok(())
    }
}
