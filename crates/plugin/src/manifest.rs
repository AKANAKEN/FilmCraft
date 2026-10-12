//! Plugin manifest and declarative capability specification.
//!
//! Declarative definitions allow both FilmCraft's UI (egui) and AI agents (MCP)
//! to inspect parameters, capabilities, and metadata without executing untrusted plugin code.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::PluginError;

/// The functional category of the plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginType {
    /// Pixel filter / frame manipulation.
    VideoEffect,
    /// Audio sample processing.
    AudioEffect,
    /// Transition between two media items.
    Transition,
    /// Workflow automation or timeline tool.
    WorkflowTool,
}

/// Sandboxing limits and hardware capabilities requested by the plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCapabilities {
    /// Maximum allowed working memory in megabytes (default 128 MB).
    #[serde(default = "default_memory_mb")]
    pub memory_limit_mb: u32,

    /// Maximum concurrent worker threads allowed (default 1).
    #[serde(default = "default_thread_count")]
    pub max_threads: u32,

    /// Whether the plugin requires WGSL GPU compute passes.
    #[serde(default)]
    pub gpu_accelerated: bool,
}

fn default_memory_mb() -> u32 {
    128
}

fn default_thread_count() -> u32 {
    1
}

impl Default for PluginCapabilities {
    fn default() -> Self {
        Self {
            memory_limit_mb: default_memory_mb(),
            max_threads: default_thread_count(),
            gpu_accelerated: false,
        }
    }
}

/// Rich metadata for human display and AI agent indexing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginMetadata {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
}

/// Supported parameter types exposed by plugins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginParamKind {
    Float {
        min: f64,
        max: f64,
        #[serde(default)]
        decimals: u8,
        #[serde(default)]
        unit: String,
    },
    Int {
        min: i64,
        max: i64,
    },
    Bool,
    ColorRgba,
    Choice {
        options: Vec<String>,
    },
    Text,
}

/// Single parameter definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginParamDef {
    pub id: String,
    pub name: String,
    pub kind: PluginParamKind,
    pub default: serde_json::Value,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub animatable: bool,
    /// Explanation of parameter semantics and expected behavior for AI agents (MCP).
    #[serde(default)]
    pub agent_hint: Option<String>,
}

/// The complete plugin declaration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub plugin_type: PluginType,
    #[serde(default)]
    pub capabilities: PluginCapabilities,
    #[serde(default)]
    pub metadata: PluginMetadata,
    /// Semantic guidance for AI agents describing artistic/technical situations to use this plugin.
    #[serde(default)]
    pub agent_hints: Vec<String>,
    /// Example natural language prompts and suggested parameter mappings for AI agents.
    #[serde(default)]
    pub agent_examples: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<PluginParamDef>,
}

impl PluginManifest {
    /// Parse a manifest from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, PluginError> {
        serde_json::from_str(json).map_err(|e| PluginError::Manifest(e.to_string()))
    }

    /// Serialize the manifest to a pretty JSON string.
    pub fn to_json(&self) -> Result<String, PluginError> {
        serde_json::to_string_pretty(self).map_err(|e| PluginError::Manifest(e.to_string()))
    }

    /// Validates parameter map values against the declared schema without panicking.
    pub fn validate_parameters(&self, values: &BTreeMap<String, serde_json::Value>) -> Result<(), PluginError> {
        for def in &self.parameters {
            if let Some(val) = values.get(&def.id) {
                match &def.kind {
                    PluginParamKind::Float { min, max, .. } => {
                        let num = val.as_f64().ok_or_else(|| PluginError::InvalidParameter {
                            name: def.id.clone(),
                            reason: "expected floating point number".into(),
                        })?;
                        if num < *min || num > *max {
                            return Err(PluginError::InvalidParameter {
                                name: def.id.clone(),
                                reason: format!("value {num} out of bounds [{min}, {max}]"),
                            });
                        }
                    }
                    PluginParamKind::Int { min, max } => {
                        let num = val.as_i64().ok_or_else(|| PluginError::InvalidParameter {
                            name: def.id.clone(),
                            reason: "expected integer".into(),
                        })?;
                        if num < *min || num > *max {
                            return Err(PluginError::InvalidParameter {
                                name: def.id.clone(),
                                reason: format!("value {num} out of bounds [{min}, {max}]"),
                            });
                        }
                    }
                    PluginParamKind::Bool => {
                        if !val.is_boolean() {
                            return Err(PluginError::InvalidParameter {
                                name: def.id.clone(),
                                reason: "expected boolean".into(),
                            });
                        }
                    }
                    PluginParamKind::Choice { options } => {
                        let s = val.as_str().ok_or_else(|| PluginError::InvalidParameter {
                            name: def.id.clone(),
                            reason: "expected string option".into(),
                        })?;
                        if !options.iter().any(|opt| opt == s) {
                            return Err(PluginError::InvalidParameter {
                                name: def.id.clone(),
                                reason: format!("option '{s}' not in allowed choices"),
                            });
                        }
                    }
                    PluginParamKind::ColorRgba => {
                        let is_array = val.as_array().map_or(false, |a| a.len() == 4);
                        if !is_array {
                            return Err(PluginError::InvalidParameter {
                                name: def.id.clone(),
                                reason: "expected 4-element RGBA array [r, g, b, a]".into(),
                            });
                        }
                    }
                    PluginParamKind::Text => {
                        if !val.is_string() {
                            return Err(PluginError::InvalidParameter {
                                name: def.id.clone(),
                                reason: "expected text string".into(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Converts this declarative plugin manifest into FilmCraft's native [`filmcraft_project::EffectDef`].
    ///
    /// Enables automatic population of the Effects Panel tree, Effect Controls rows, and keyframe editors
    /// without requiring custom GUI code.
    pub fn to_effect_def(&self) -> filmcraft_project::EffectDef {
        let id_static: &'static str = Box::leak(self.id.clone().into_boxed_str());
        let name_static: &'static str = Box::leak(self.name.clone().into_boxed_str());
        let category: &'static [&'static str] = match self.plugin_type {
            PluginType::VideoEffect => &["Video Effects", "Plugins"],
            PluginType::AudioEffect => &["Audio Effects", "Plugins"],
            PluginType::Transition => &["Video Transitions", "Plugins"],
            PluginType::WorkflowTool => &["Tools", "Plugins"],
        };

        let kind = match self.plugin_type {
            PluginType::VideoEffect => filmcraft_project::EffectKind::Video,
            PluginType::AudioEffect => filmcraft_project::EffectKind::Audio,
            PluginType::Transition => filmcraft_project::EffectKind::VideoTransition,
            PluginType::WorkflowTool => filmcraft_project::EffectKind::Video,
        };

        let params: Vec<filmcraft_project::ParamDef> = self
            .parameters
            .iter()
            .map(|p| {
                let p_id: &'static str = Box::leak(p.id.clone().into_boxed_str());
                let p_name: &'static str = Box::leak(p.name.clone().into_boxed_str());
                let unit: &'static str = match &p.kind {
                    PluginParamKind::Float { unit, .. } => Box::leak(unit.clone().into_boxed_str()),
                    _ => "",
                };

                let kind = match &p.kind {
                    PluginParamKind::Float {
                        min,
                        max,
                        decimals,
                        ..
                    } => filmcraft_project::ParamKind::Float {
                        min: *min,
                        max: *max,
                        soft_min: *min,
                        soft_max: *max,
                        unit,
                        decimals: *decimals,
                    },
                    PluginParamKind::Int { min, max } => filmcraft_project::ParamKind::Float {
                        min: *min as f64,
                        max: *max as f64,
                        soft_min: *min as f64,
                        soft_max: *max as f64,
                        unit: "",
                        decimals: 0,
                    },
                    PluginParamKind::Bool => filmcraft_project::ParamKind::Bool,
                    PluginParamKind::ColorRgba => filmcraft_project::ParamKind::Color,
                    PluginParamKind::Choice { options } => {
                        let opts: Vec<&'static str> = options
                            .iter()
                            .map(|o| -> &'static str { Box::leak(o.clone().into_boxed_str()) })
                            .collect();
                        let opts_slice: &'static [&'static str] = Box::leak(opts.into_boxed_slice());
                        filmcraft_project::ParamKind::Choice(opts_slice)
                    }
                    PluginParamKind::Text => filmcraft_project::ParamKind::Text,
                };

                let default = match &p.kind {
                    PluginParamKind::Float { .. } => {
                        filmcraft_project::ParamValue::Float(p.default.as_f64().unwrap_or(0.0))
                    }
                    PluginParamKind::Int { .. } => {
                        filmcraft_project::ParamValue::Float(p.default.as_f64().unwrap_or(0.0))
                    }
                    PluginParamKind::Bool => {
                        filmcraft_project::ParamValue::Bool(p.default.as_bool().unwrap_or(false))
                    }
                    PluginParamKind::ColorRgba => {
                        let mut rgba = [1.0, 1.0, 1.0, 1.0];
                        if let Some(arr) = p.default.as_array() {
                            if arr.len() == 4 {
                                for (i, v) in arr.iter().take(4).enumerate() {
                                    if let Some(num) = v.as_f64() {
                                        rgba[i] = num as f32;
                                    }
                                }
                            }
                        }
                        filmcraft_project::ParamValue::Color(rgba)
                    }
                    PluginParamKind::Choice { options } => {
                        let idx = if let Some(s) = p.default.as_str() {
                            options.iter().position(|o| o == s).unwrap_or(0) as u32
                        } else {
                            p.default.as_u64().unwrap_or(0) as u32
                        };
                        filmcraft_project::ParamValue::Choice(idx)
                    }
                    PluginParamKind::Text => {
                        filmcraft_project::ParamValue::Text(p.default.as_str().unwrap_or("").to_string())
                    }
                };

                filmcraft_project::ParamDef {
                    id: p_id,
                    label: p_name,
                    kind,
                    default,
                    animatable: p.animatable,
                    group: None,
                }
            })
            .collect();

        filmcraft_project::EffectDef {
            id: id_static,
            name: name_static,
            kind,
            category,
            params,
            intrinsic: false,
            accelerated: self.capabilities.gpu_accelerated,
            float32: true,
            yuv: false,
        }
    }
}
