//! Plugin loader, directory scanner, and session integration for FilmCraft.
//!
//! Automatically discovers sandboxed `.wasm` plugins and registers them with
//! FilmCraft's project model ([`filmcraft_project::register_dynamic_effect`])
//! so they appear in the Effects Panel and Inspector without manual GUI wiring.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use filmcraft_plugin::poc::GrayscaleEffect;
use filmcraft_plugin::registry::PluginRegistry;
use filmcraft_plugin::traits::FilmCraftPlugin;
use filmcraft_plugin::wasm_host::{WasmHostEngine, WasmVideoEffectProxy};

/// Information about an installed plugin for display in `Settings ▸ Plugins`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InstalledPluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub path: Option<PathBuf>,
    pub enabled: bool,
}

/// Central plugin manager held by the engine session.
#[derive(Clone)]
pub struct PluginManager {
    registry: Arc<RwLock<PluginRegistry>>,
    wasm_engine: Option<WasmHostEngine>,
    installed: Arc<RwLock<Vec<InstalledPluginInfo>>>,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginManager {
    /// Creates a new PluginManager and registers built-in reference plugins.
    pub fn new() -> Self {
        let registry = Arc::new(RwLock::new(PluginRegistry::new()));
        let wasm_engine = match WasmHostEngine::new() {
            Ok(engine) => Some(engine),
            Err(err) => {
                log::warn!("Failed to initialize Wasmtime JIT engine: {err}; WASM plugins will not be available");
                None
            }
        };

        let manager = Self {
            registry,
            wasm_engine,
            installed: Arc::new(RwLock::new(Vec::new())),
        };

        // Always register reference built-in plugins
        manager.load_builtins();
        manager.scan_default_locations();
        manager
    }

    /// Access the underlying thread-safe registry.
    pub fn registry(&self) -> Arc<RwLock<PluginRegistry>> {
        self.registry.clone()
    }

    /// Register built-in reference plugins (e.g., Grayscale PoC) into both the registry and the Effects Panel tree.
    fn load_builtins(&self) {
        let grayscale = GrayscaleEffect::new();
        let manifest = grayscale.manifest().clone();

        // Register into project effect definitions so the UI displays it under Video Effects ▸ Plugins
        let effect_def = manifest.to_effect_def();
        filmcraft_project::register_dynamic_effect(effect_def);

        // Register into execution registry
        if let Ok(mut reg) = self.registry.write() {
            let _ = reg.register_video_effect(grayscale);
        }

        if let Ok(mut installed) = self.installed.write() {
            installed.push(InstalledPluginInfo {
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                description: manifest.metadata.description,
                author: manifest.metadata.authors.join(", "),
                path: None,
                enabled: true,
            });
        }

        if let Ok(reg) = self.registry.read() {
            filmcraft_render::plugin_bridge::set_render_plugin_registry(reg.clone());
        }
    }

    /// Scans standard plugin locations on disk (e.g. `./plugins` or `FILMCRAFT_PLUGINS_DIR`).
    pub fn scan_default_locations(&self) {
        if let Some(dir) = std::env::var_os("FILMCRAFT_PLUGINS_DIR") {
            self.scan_directory(Path::new(&dir));
        }
        let local_plugins = Path::new("plugins");
        if local_plugins.is_dir() {
            self.scan_directory(local_plugins);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                let app_plugins = exe_dir.join("plugins");
                if app_plugins != local_plugins && app_plugins.is_dir() {
                    self.scan_directory(&app_plugins);
                }
            }
        }
    }

    /// Scan a directory on disk for `.wasm` plugins.
    pub fn scan_directory(&self, dir: &Path) {
        if !dir.is_dir() {
            return;
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(err) => {
                log::warn!("Failed to read plugins directory {}: {err}", dir.display());
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("wasm") {
                self.load_wasm_file(&path);
            }
        }
    }

    /// Loads a single `.wasm` plugin file into the sandbox and UI tree.
    pub fn load_wasm_file(&self, path: &Path) {
        let Some(ref wasm_engine) = self.wasm_engine else {
            log::warn!("WASM engine is unavailable; cannot load plugin at {}", path.display());
            return;
        };

        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(err) => {
                log::warn!("Failed to read plugin at {}: {err}", path.display());
                return;
            }
        };

        match WasmVideoEffectProxy::from_wasm_bytes(wasm_engine, &bytes) {
            Ok(plugin) => {
                let manifest = plugin.manifest().clone();

                // Register into UI Effect Tree
                let effect_def = manifest.to_effect_def();
                filmcraft_project::register_dynamic_effect(effect_def);

                // Register into execution registry
                if let Ok(mut reg) = self.registry.write() {
                    let _ = reg.register_video_effect(plugin);
                }

                if let Ok(mut installed) = self.installed.write() {
                    installed.push(InstalledPluginInfo {
                        id: manifest.id,
                        name: manifest.name,
                        version: manifest.version,
                        description: manifest.metadata.description,
                        author: manifest.metadata.authors.join(", "),
                        path: Some(path.to_path_buf()),
                        enabled: true,
                    });
                }

                if let Ok(reg) = self.registry.read() {
                    filmcraft_render::plugin_bridge::set_render_plugin_registry(reg.clone());
                }

                log::info!("Successfully loaded sandboxed plugin: {}", path.display());
            }
            Err(err) => {
                log::warn!("Failed to instantiate plugin {}: {err}", path.display());
            }
        }
    }

    /// Returns list of all installed plugins for GUI settings display.
    pub fn list_installed(&self) -> Vec<InstalledPluginInfo> {
        self.installed.read().map(|l| l.clone()).unwrap_or_default()
    }
}
