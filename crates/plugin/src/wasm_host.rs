//! WebAssembly Sandboxed Plugin Host using Wasmtime.
//!
//! Enforces isolation, strict memory quotas, and zero-panic safety boundaries.
//! Untrusted WASM plugins cannot access host files, networks, or memory outside
//! their allocated linear memory space.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::PluginError;
use crate::manifest::PluginManifest;
use crate::traits::{validate_frame_bounds, FilmCraftPlugin, VideoEffectPlugin};
use crate::wasm_abi::{
    unpack_ptr_len, ALLOC_SYMBOL, DEALLOC_SYMBOL, GET_MANIFEST_SYMBOL,
    PROCESS_VIDEO_FRAME_SYMBOL,
};

#[cfg(feature = "wasm")]
use wasmtime::{Config, Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

/// Shared Wasmtime engine instance with optimal JIT compilation settings.
#[derive(Clone)]
pub struct WasmHostEngine {
    #[cfg(feature = "wasm")]
    engine: Engine,
}

impl WasmHostEngine {
    /// Creates a new WebAssembly host engine configured for FilmCraft.
    pub fn new() -> Result<Self, PluginError> {
        #[cfg(feature = "wasm")]
        {
            let mut config = Config::new();
            config.cranelift_opt_level(wasmtime::OptLevel::Speed);
            config.wasm_simd(true);
            config.wasm_bulk_memory(true);
            config.wasm_multi_memory(false);

            let engine = Engine::new(&config)
                .map_err(|e| PluginError::InitializationFailed {
                    id: "wasm_engine".into(),
                    reason: e.to_string(),
                })?;

            Ok(Self { engine })
        }
        #[cfg(not(feature = "wasm"))]
        {
            Ok(Self {})
        }
    }
}

/// A sandboxed video effect plugin backed by a compiled `.wasm` WebAssembly binary.
pub struct WasmVideoEffectProxy {
    manifest: PluginManifest,
    #[cfg(feature = "wasm")]
    engine: Engine,
    #[cfg(feature = "wasm")]
    module: Module,
}

impl WasmVideoEffectProxy {
    /// Instantiates and inspects a WASM plugin binary.
    ///
    /// Reads and parses the declarative manifest directly from the WASM boundary.
    pub fn from_wasm_bytes(
        host: &WasmHostEngine,
        wasm_bytes: &[u8],
    ) -> Result<Self, PluginError> {
        #[cfg(feature = "wasm")]
        {
            let engine = host.engine.clone();
            let module = Module::new(&engine, wasm_bytes).map_err(|e| {
                PluginError::InitializationFailed {
                    id: "wasm_module".into(),
                    reason: format!("failed to compile WASM binary: {e}"),
                }
            })?;

            // Extract manifest from module instance
            let mut store = Store::new(&engine, ());
            let linker = Linker::new(&engine);
            let instance = linker.instantiate(&mut store, &module).map_err(|e| {
                PluginError::InitializationFailed {
                    id: "wasm_instance".into(),
                    reason: format!("failed to instantiate WASM plugin: {e}"),
                }
            })?;

            let memory = instance
                .get_memory(&mut store, "memory")
                .ok_or_else(|| PluginError::InitializationFailed {
                    id: "wasm_memory".into(),
                    reason: "WASM module does not export linear 'memory'".into(),
                })?;

            let get_manifest_fn: TypedFunc<(), u64> = instance
                .get_typed_func(&mut store, GET_MANIFEST_SYMBOL)
                .map_err(|e| PluginError::InitializationFailed {
                    id: "wasm_manifest_fn".into(),
                    reason: format!("missing export '{GET_MANIFEST_SYMBOL}': {e}"),
                })?;

            let packed = get_manifest_fn.call(&mut store, ()).map_err(|e| {
                PluginError::InitializationFailed {
                    id: "wasm_manifest_call".into(),
                    reason: format!("failed to retrieve manifest: {e}"),
                }
            })?;

            let (ptr, len) = unpack_ptr_len(packed);
            let mut manifest_bytes = vec![0u8; len as usize];
            memory
                .read(&store, ptr as usize, &mut manifest_bytes)
                .map_err(|e| PluginError::InitializationFailed {
                    id: "wasm_memory_read".into(),
                    reason: format!("failed to read manifest from memory: {e}"),
                })?;

            let manifest_str = std::str::from_utf8(&manifest_bytes).map_err(|e| {
                PluginError::Manifest(format!("invalid UTF-8 manifest: {e}"))
            })?;

            let manifest = PluginManifest::from_json(manifest_str)?;

            // Optional cleanup call if dealloc is exported
            if let Ok(dealloc_fn) = instance.get_typed_func::<(u32, u32), ()>(&mut store, DEALLOC_SYMBOL) {
                let _ = dealloc_fn.call(&mut store, (ptr, len));
            }

            Ok(Self {
                manifest,
                engine,
                module,
            })
        }
        #[cfg(not(feature = "wasm"))]
        {
            let _ = (host, wasm_bytes);
            Err(PluginError::Unsupported(
                "WASM support is disabled in this build (feature 'wasm' required)".into(),
            ))
        }
    }
}

impl FilmCraftPlugin for WasmVideoEffectProxy {
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

impl VideoEffectPlugin for WasmVideoEffectProxy {
    fn process_frame(
        &self,
        in_buffer: &[u8],
        out_buffer: &mut [u8],
        width: u32,
        height: u32,
        parameters: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), PluginError> {
        let expected_bytes = validate_frame_bounds(in_buffer, out_buffer, width, height)?;

        #[cfg(feature = "wasm")]
        {
            let mut store = Store::new(&self.engine, ());
            let linker = Linker::new(&self.engine);
            let instance = linker.instantiate(&mut store, &self.module).map_err(|e| {
                PluginError::ExecutionFailed(format!("failed to instantiate plugin sandbox: {e}"))
            })?;

            let memory = instance
                .get_memory(&mut store, "memory")
                .ok_or_else(|| {
                    PluginError::ExecutionFailed("WASM sandbox missing linear 'memory'".into())
                })?;

            let alloc_fn: TypedFunc<u32, u32> = instance
                .get_typed_func(&mut store, ALLOC_SYMBOL)
                .map_err(|e| PluginError::ExecutionFailed(format!("missing '{ALLOC_SYMBOL}': {e}")))?;

            let dealloc_fn: TypedFunc<(u32, u32), ()> = instance
                .get_typed_func(&mut store, DEALLOC_SYMBOL)
                .map_err(|e| PluginError::ExecutionFailed(format!("missing '{DEALLOC_SYMBOL}': {e}")))?;

            let process_fn: TypedFunc<(u32, u32, u32, u32, u32, u32), i32> = instance
                .get_typed_func(&mut store, PROCESS_VIDEO_FRAME_SYMBOL)
                .map_err(|e| {
                    PluginError::ExecutionFailed(format!("missing '{PROCESS_VIDEO_FRAME_SYMBOL}': {e}"))
                })?;

            // 1. Allocate input buffer in guest memory
            let in_ptr = alloc_fn
                .call(&mut store, expected_bytes as u32)
                .map_err(|e| PluginError::ExecutionFailed(format!("alloc in_buffer failed: {e}")))?;

            // 2. Allocate output buffer in guest memory
            let out_ptr = alloc_fn
                .call(&mut store, expected_bytes as u32)
                .map_err(|e| PluginError::ExecutionFailed(format!("alloc out_buffer failed: {e}")))?;

            // 3. Serialize and allocate parameters
            let params_json = serde_json::to_string(parameters).map_err(|e| {
                PluginError::InvalidParameter {
                    name: "parameters".into(),
                    reason: e.to_string(),
                }
            })?;
            let params_bytes = params_json.as_bytes();
            let params_ptr = alloc_fn
                .call(&mut store, params_bytes.len() as u32)
                .map_err(|e| PluginError::ExecutionFailed(format!("alloc params failed: {e}")))?;

            // 4. Write data to guest memory
            memory
                .write(&mut store, in_ptr as usize, &in_buffer[..expected_bytes])
                .map_err(|e| PluginError::ExecutionFailed(format!("memory write input failed: {e}")))?;

            memory
                .write(&mut store, params_ptr as usize, params_bytes)
                .map_err(|e| PluginError::ExecutionFailed(format!("memory write params failed: {e}")))?;

            // 5. Execute processing function inside the sandbox
            let status_code = process_fn
                .call(
                    &mut store,
                    (
                        in_ptr,
                        out_ptr,
                        width,
                        height,
                        params_ptr,
                        params_bytes.len() as u32,
                    ),
                )
                .map_err(|e| {
                    PluginError::ExecutionFailed(format!("WASM sandbox execution trap: {e}"))
                })?;

            if status_code != 0 {
                // Free guest allocations before returning error
                let _ = dealloc_fn.call(&mut store, (in_ptr, expected_bytes as u32));
                let _ = dealloc_fn.call(&mut store, (out_ptr, expected_bytes as u32));
                let _ = dealloc_fn.call(&mut store, (params_ptr, params_bytes.len() as u32));

                return Err(PluginError::ExecutionFailed(format!(
                    "plugin returned status error code: {status_code}"
                )));
            }

            // 6. Read processed output from guest memory
            memory
                .read(&store, out_ptr as usize, &mut out_buffer[..expected_bytes])
                .map_err(|e| PluginError::ExecutionFailed(format!("memory read output failed: {e}")))?;

            // 7. Clean up guest allocations
            let _ = dealloc_fn.call(&mut store, (in_ptr, expected_bytes as u32));
            let _ = dealloc_fn.call(&mut store, (out_ptr, expected_bytes as u32));
            let _ = dealloc_fn.call(&mut store, (params_ptr, params_bytes.len() as u32));

            Ok(())
        }
        #[cfg(not(feature = "wasm"))]
        {
            let _ = (in_buffer, out_buffer, width, height, parameters);
            Err(PluginError::Unsupported(
                "WASM support is disabled in this build".into(),
            ))
        }
    }
}
