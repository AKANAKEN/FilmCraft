//! Registry and dispatch manager for FilmCraft plugins.
//!
//! Provides lookup, lifecycle management, and thread-safe execution dispatching
//! for both native/PoC plugins and future WASM-sandboxed plugins.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::PluginError;
use crate::manifest::PluginManifest;
use crate::traits::{FilmCraftPlugin, VideoEffectPlugin};

/// Thread-safe registry holding available plugins.
#[derive(Default, Clone)]
pub struct PluginRegistry {
    video_effects: BTreeMap<String, Arc<dyn VideoEffectPlugin>>,
}

impl PluginRegistry {
    /// Create a new empty plugin registry.
    pub fn new() -> Self {
        Self {
            video_effects: BTreeMap::new(),
        }
    }

    /// Register a video effect plugin.
    pub fn register_video_effect<P: VideoEffectPlugin + 'static>(&mut self, plugin: P) -> Result<(), PluginError> {
        let id = plugin.id().to_string();
        if self.video_effects.contains_key(&id) {
            return Err(PluginError::InitializationFailed {
                id: id.clone(),
                reason: "plugin with this id already registered".into(),
            });
        }
        self.video_effects.insert(id, Arc::new(plugin));
        Ok(())
    }

    /// Retrieve a registered video effect plugin by id.
    pub fn get_video_effect(&self, id: &str) -> Option<Arc<dyn VideoEffectPlugin>> {
        self.video_effects.get(id).cloned()
    }

    /// List all registered plugin manifests for UI inspection and AI MCP discovery.
    pub fn list_manifests(&self) -> Vec<&PluginManifest> {
        self.video_effects.values().map(|p| p.manifest()).collect()
    }

    /// Safely process a frame using a registered plugin without risk of crashing the engine.
    pub fn process_video_frame(
        &self,
        id: &str,
        in_buffer: &[u8],
        out_buffer: &mut [u8],
        width: u32,
        height: u32,
        parameters: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), PluginError> {
        let effect = self
            .video_effects
            .get(id)
            .ok_or_else(|| PluginError::NotFound(id.to_string()))?;

        // Validate parameters against the manifest schema first.
        effect.manifest().validate_parameters(parameters)?;

        // Execute the effect.
        effect.process_frame(in_buffer, out_buffer, width, height, parameters)
    }

    /// Convenience execution entry point taking parameters as a JSON string.
    ///
    /// Ideal for automation bridges, MCP agent dispatch, and decoupled render pipelines.
    pub fn execute_video_effect_json(
        &self,
        id: &str,
        in_buffer: &[u8],
        out_buffer: &mut [u8],
        width: u32,
        height: u32,
        parameters_json: &str,
    ) -> Result<(), PluginError> {
        let params: BTreeMap<String, serde_json::Value> = if parameters_json.trim().is_empty() {
            BTreeMap::new()
        } else {
            serde_json::from_str(parameters_json).map_err(|e| PluginError::InvalidParameter {
                name: "parameters_json".into(),
                reason: e.to_string(),
            })?
        };

        self.process_video_frame(id, in_buffer, out_buffer, width, height, &params)
    }
}
