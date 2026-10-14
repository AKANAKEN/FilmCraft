# GPU export readback

`ExportRenderer::render` returns the linear premultiplied float accumulator. The
optional `render_rgba8` route packs opaque SDR pixels on the GPU using the exact
4096-entry `filmcraft-color` lookup table, then reads back four bytes per pixel.
It avoids the eight-byte half-float readback and CPU expansion/conversion. This
is not zero-copy encoding; the encoder still receives a CPU buffer.

The export pipeline selects it only for SDR without retained alpha, additional
placement/resampling, or video export effects. Timeline layers, their effects,
and burned captions are composited normally. HDR, alpha, export overlays and
video limiting retain the float route. Renderers that do not implement the new
optional hook keep their existing behavior. Invalid output dimensions or lengths
fall back to the float route. Device errors keep the existing CPU fallback.

Packing resources are initialized on demand. Output/readback buffers are reused
and constrained by checked dimensions and device limits. The CLI explicitly
enables native wgpu backends instead of depending on desktop feature unification.

## Verification

- `cargo test --release -p filmcraft-gpu --lib`: GPU compositor coverage, including
  exact comparison with the float export conversion, odd edges and every finite
  binary16 pattern.
- `cargo test --release -p filmcraft-gpu --test packed_pipeline`: actual PNG export
  with stacked layers, motion, opacity, blur and captions; exact bytes against
  the float GPU route; alpha, export effects and resampling retain that route.
- `cargo test --release -p filmcraft-export --lib packed_tests`: eligibility and
  malformed renderer output fallback.

GPU tests explicitly skip when no adapter is available. They need at least one
run on a real GPU as well as headless CI.

## Measured scope and limitation

A private 468.734-second, 1920x1080, 30000/1001 production sequence was exported
in A/B/B/A order on Windows with a Ryzen 5 4500 and RTX 3080 (driver 616.92).
Both binaries used commit `cee1feb`, native backends, thin LTO and one codegen
unit, with H.264 Main NVENC, 10/12 Mbps VBR, burned captions and -23 LUFS audio.
No builds ran during those four exports. Reference: 207.720/206.583 seconds;
packed: 158.153/157.099 seconds (23.9% lower mean time). This is one machine and
one sequence, not a cross-platform speed guarantee.

Hardware decoding produced intermittent dark source frames in both variants
and differences between repeated reference exports. That configuration has not
passed visual QA. Root cause is unresolved; decode success and mean SSIM alone
do not establish quality. See [issue #1724](https://github.com/storytold/filmcraft/issues/1724)
for the evidence and the unresolved reproduction limitation. With Media Foundation decoding disabled, reference
and packed exports of the same full sequence were identical by SHA-256. Both
retained -23.0 LUFS, stereo 48 kHz audio and all 14,048 video frames. The patch
does not change decoder preferences or work around that separate problem.

The implementation uses FilmCraft's own color conversion and public wgpu APIs;
no proprietary implementation, assets, presets or shaders are incorporated.
