# FilmCraft Plugin SDK (`filmcraft-plugin-sdk`)

The official Rust SDK for building sandboxed, high-performance WebAssembly (WASM) plugins for [FilmCraft](https://github.com/storytold/filmcraft).

## Features

- **Safe Rust:** Zero `unsafe` required from plugin authors.
- **Panic Containment:** Double-layer `catch_unwind` and Wasmtime trap isolation ensures bad code never crashes the editor.
- **Ergonomic API:** Work with [`VideoFrame`](src/frame.rs) and mutate pixels with zero FFI boilerplate.
- **Single-Macro Export:** [`export_video_effect!`](src/macros.rs) defines all required C-ABI and memory management functions automatically.

## Quickstart

### 1. `Cargo.toml`

```toml
[package]
name = "my-filmcraft-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
filmcraft-plugin-sdk = { path = "../../crates/plugin-sdk" }
serde_json = "1.0"
```

### 2. `src/lib.rs`

```rust
use filmcraft_plugin_sdk::prelude::*;

const MANIFEST: &str = r#"{
  "id": "my.cool.effect",
  "name": "Cool Effect",
  "version": "0.1.0",
  "plugin_type": "video_effect",
  "parameters": []
}"#;

#[derive(Default)]
pub struct CoolEffect;

impl VideoEffect for CoolEffect {
    fn process_frame(
        &mut self,
        frame: &mut VideoFrame,
        _params: &serde_json::Value,
    ) -> Result<(), SdkError> {
        frame.for_each_pixel_mut(|p| {
            p.r = 255 - p.r;
        });
        Ok(())
    }
}

// Exports all WASM FFI symbols automatically
export_video_effect!(CoolEffect, MANIFEST);
```

### 3. Compile to WebAssembly

```bash
cargo build --target wasm32-unknown-unknown --release
```

The resulting `target/wasm32-unknown-unknown/release/my_filmcraft_plugin.wasm` file is ready to drop into FilmCraft!
