# ADR 0002: Sandboxed WebAssembly (WASM) Plugin Architecture in `crates/plugin`

- **Status:** proposed (2026-10-11)
- **Issue:** G11 (Plugin hosting, [gaps.md](../gaps.md#g11-no-plugin-hosting))

## Context

FilmCraft is a professional, pure-Rust non-linear video editor with a strict **"Never Crash"** rule ([AGENTS.md](../../AGENTS.md) §0) and workspace-wide `unsafe_code = "forbid"` (with the sole exception of OS hardware video decoders in `crates/platform`, [ADR 0001](0001-platform-ffi.md)).

Third-party plugins are essential for professional workflows, but traditional video editor plugin architectures have severe flaws:
1. **Host-crashing native dynamic libraries (.dll, .so, .dylib):** Loading arbitrary untrusted C/C++ dynamic libraries into the host process violates FilmCraft's rule 0. A single segfault, null-pointer dereference, or panic in a third-party plugin instantly kills the editor, destroying unsaved user projects.
2. **Adobe-style proprietary ecosystems (CEP, UXP, ExtendScript):** Embedded Chromium webviews with Node.js bindings or legacy ECMAScript runtimes introduce extreme bloat, security liabilities, memory overhead, and licensing entanglements.
3. **IPC / Socket Overhead:** Streaming uncompressed 4K video frames over JSON-RPC or inter-process sockets causes unacceptable latency and deserialization overhead.

## Decision

Implement the plugin ecosystem in `crates/plugin` (`filmcraft-plugin`) based on a **WebAssembly (WASM) Sandboxed Runtime** using Wasmtime.

### Core Architecture

1. **Strict Sandboxing & Never-Crash Guarantee:**
   - Plugins run in a secure, isolated WASM linear memory sandbox.
   - Out-of-bounds access, division by zero, or internal panics within a plugin trigger a trapped `Result::Err(PluginError::ExecutionFailed)`, never crashing the host.
   - Memory quotas (`memory_limit_mb`) and thread bounds are enforced upfront.
2. **Zero Proprietary/Adobe Dependency:**
   - 100% clean-room open-standard ABI.
   - Declarative `plugin.toml` / JSON manifest schema representing parameters (`Float`, `Int`, `Bool`, `ColorRgba`, `Choice`, `Text`).
   - Native integration with FilmCraft's `egui` parameter inspectors and AI agent (MCP) automation schemas.
3. **Standardized C-WASM ABI:**
   - `filmcraft_alloc(size: u32) -> u32`
   - `filmcraft_dealloc(ptr: u32, size: u32) -> ()`
   - `filmcraft_get_manifest() -> u64` (packed `ptr` and `len`)
   - `filmcraft_process_video_frame(in_ptr: u32, out_ptr: u32, width: u32, height: u32, params_ptr: u32, params_len: u32) -> i32`
4. **Gradual In-Process & Host Proxy Integration:**
   - Plugins implement the safe `VideoEffectPlugin` and `FilmCraftPlugin` traits.
   - Built-in effects can run in-process without sandbox overhead, while external untrusted third-party plugins run behind `WasmVideoEffectProxy`.
