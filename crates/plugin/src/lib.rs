//! # filmcraft-plugin
//!
//! Sandboxed plugin specification, declarative manifests, and host runtime interfaces
//! for FilmCraft.
//!
//! Designed specifically to adhere to FilmCraft's strict "Never crash" rules:
//! * Memory and thread limits are declared up front.
//! * Every hostile input (dimensions, buffer lengths, schema mismatches) is validated safely.
//! * No panics, unwrap, or unsafe code.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod error;
pub mod manifest;
pub mod poc;
pub mod registry;
pub mod traits;
pub mod wasm_abi;
pub mod wasm_host;

pub use error::PluginError;
pub use manifest::{
    PluginCapabilities, PluginManifest, PluginMetadata, PluginParamDef, PluginParamKind, PluginType,
};
pub use poc::GrayscaleEffect;
pub use registry::PluginRegistry;
pub use traits::{validate_frame_bounds, AudioEffectPlugin, FilmCraftPlugin, VideoEffectPlugin};
pub use wasm_abi::WasmStatusCode;
pub use wasm_host::{WasmHostEngine, WasmVideoEffectProxy};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_manifest_serialization() {
        let effect = GrayscaleEffect::new();
        let json = effect.manifest().to_json().expect("manifest serialization failed");
        let parsed = PluginManifest::from_json(&json).expect("manifest parsing failed");

        assert_eq!(parsed.id, "craft.builtin.poc_grayscale");
        assert_eq!(parsed.plugin_type, PluginType::VideoEffect);
        assert_eq!(parsed.parameters.len(), 1);
        assert_eq!(parsed.parameters[0].id, "amount");
    }

    #[test]
    fn test_grayscale_processing_full() {
        let effect = GrayscaleEffect::new();
        let width = 2;
        let height = 2;
        // Pure red, green, blue, and white pixels
        let in_pixels: Vec<u8> = vec![
            255, 0, 0, 255,   // Red
            0, 255, 0, 255,   // Green
            0, 0, 255, 255,   // Blue
            255, 255, 255, 255 // White
        ];
        let mut out_pixels = vec![0u8; in_pixels.len()];

        let mut params = BTreeMap::new();
        params.insert("amount".to_string(), serde_json::json!(1.0));

        let res = effect.process_frame(&in_pixels, &mut out_pixels, width, height, &params);
        assert!(res.is_ok());

        // Red luma: 0.2126 * 255 ~= 54
        assert_eq!(out_pixels[0], 54);
        assert_eq!(out_pixels[1], 54);
        assert_eq!(out_pixels[2], 54);
        assert_eq!(out_pixels[3], 255); // Alpha preserved

        // White pixel remains white: ~255
        assert_eq!(out_pixels[12], 255);
        assert_eq!(out_pixels[13], 255);
        assert_eq!(out_pixels[14], 255);
        assert_eq!(out_pixels[15], 255);
    }

    #[test]
    fn test_grayscale_processing_zero_amount() {
        let effect = GrayscaleEffect::new();
        let width = 1;
        let height = 1;
        let in_pixels: Vec<u8> = vec![120, 60, 200, 255];
        let mut out_pixels = vec![0u8; 4];

        let mut params = BTreeMap::new();
        params.insert("amount".to_string(), serde_json::json!(0.0));

        let res = effect.process_frame(&in_pixels, &mut out_pixels, width, height, &params);
        assert!(res.is_ok());

        // With amount = 0.0, output matches input exactly
        assert_eq!(out_pixels, in_pixels);
    }

    #[test]
    fn test_hostile_zero_dimensions() {
        let effect = GrayscaleEffect::new();
        let in_pixels = vec![0u8; 16];
        let mut out_pixels = vec![0u8; 16];
        let params = BTreeMap::new();

        let res = effect.process_frame(&in_pixels, &mut out_pixels, 0, 4, &params);
        assert_eq!(res, Err(PluginError::InvalidDimensions { width: 0, height: 4 }));

        let res2 = effect.process_frame(&in_pixels, &mut out_pixels, 4, 0, &params);
        assert_eq!(res2, Err(PluginError::InvalidDimensions { width: 4, height: 0 }));
    }

    #[test]
    fn test_hostile_buffer_size_mismatch() {
        let effect = GrayscaleEffect::new();
        let in_pixels = vec![0u8; 8]; // Needs 16 bytes for 2x2 RGBA
        let mut out_pixels = vec![0u8; 16];
        let params = BTreeMap::new();

        let res = effect.process_frame(&in_pixels, &mut out_pixels, 2, 2, &params);
        assert_eq!(
            res,
            Err(PluginError::BufferSizeMismatch {
                expected: 16,
                actual: 8
            })
        );
    }

    #[test]
    fn test_parameter_validation() {
        let effect = GrayscaleEffect::new();
        let mut valid_params = BTreeMap::new();
        valid_params.insert("amount".to_string(), serde_json::json!(0.75));
        assert!(effect.manifest().validate_parameters(&valid_params).is_ok());

        let mut out_of_bounds = BTreeMap::new();
        out_of_bounds.insert("amount".to_string(), serde_json::json!(1.5));
        assert!(effect.manifest().validate_parameters(&out_of_bounds).is_err());

        let mut invalid_type = BTreeMap::new();
        invalid_type.insert("amount".to_string(), serde_json::json!("not_a_number"));
        assert!(effect.manifest().validate_parameters(&invalid_type).is_err());
    }

    #[test]
    fn test_plugin_registry_lifecycle() {
        let mut registry = PluginRegistry::new();
        let effect = GrayscaleEffect::new();
        assert!(registry.register_video_effect(effect.clone()).is_ok());

        // Duplicate registration should fail gracefully with PluginError
        assert!(registry.register_video_effect(effect).is_err());

        // Manifest lookup
        let manifests = registry.list_manifests();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].id, "craft.builtin.poc_grayscale");

        // Processing via registry
        let in_pixels = vec![255, 0, 0, 255];
        let mut out_pixels = vec![0u8; 4];
        let mut params = BTreeMap::new();
        params.insert("amount".to_string(), serde_json::json!(1.0));

        let res = registry.process_video_frame("craft.builtin.poc_grayscale", &in_pixels, &mut out_pixels, 1, 1, &params);
        assert!(res.is_ok());
        assert_eq!(out_pixels[0], 54);

        // Processing via registry JSON helper
        let res_json = registry.execute_video_effect_json("craft.builtin.poc_grayscale", &in_pixels, &mut out_pixels, 1, 1, r#"{"amount": 1.0}"#);
        assert!(res_json.is_ok());
        assert_eq!(out_pixels[0], 54);

        // Not found plugin execution
        let not_found_res = registry.process_video_frame("non.existent.plugin", &in_pixels, &mut out_pixels, 1, 1, &params);
        assert_eq!(not_found_res, Err(PluginError::NotFound("non.existent.plugin".to_string())));
    }
}
