//! # filmcraft-plugin-sdk
//!
//! Developer-facing SDK for building high-performance, sandboxed WebAssembly
//! plugins for FilmCraft.
//!
//! Exposes the safe [`VideoFrame`] abstraction and the declarative
//! [`export_video_effect!`] macro to handle WASM FFI, allocation, and
//! double-layer panic containment automatically.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod audio_block;
pub mod effect;
pub mod error;
pub mod ffi;
pub mod frame;
pub mod macros;
pub mod prelude;

pub use audio_block::AudioBlock;
pub use effect::{AudioEffect, VideoEffect};
pub use error::SdkError;
pub use frame::{PixelRgba, VideoFrame};

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct InvertEffect;

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
            });
            Ok(())
        }
    }

    #[test]
    fn test_video_frame_pixel_manipulation() {
        let mut buffer = vec![
            10, 20, 30, 255,   // Pixel 0
            100, 150, 200, 255 // Pixel 1
        ];

        let mut frame = VideoFrame::new(&mut buffer, 2, 1).expect("frame creation failed");
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.height(), 1);

        let mut effect = InvertEffect::default();
        let res = effect.process_frame(&mut frame, &serde_json::Value::Null);
        assert!(res.is_ok());

        assert_eq!(buffer[0], 245);
        assert_eq!(buffer[1], 235);
        assert_eq!(buffer[2], 225);
        assert_eq!(buffer[3], 255); // Alpha untouched

        assert_eq!(buffer[4], 155);
        assert_eq!(buffer[5], 105);
        assert_eq!(buffer[6], 55);
        assert_eq!(buffer[7], 255);
    }

    #[test]
    fn test_insufficient_buffer_rejection() {
        let mut buffer = vec![0u8; 10]; // Requires 16 bytes for 2x2 RGBA
        assert!(VideoFrame::new(&mut buffer, 2, 2).is_none());
    }

    #[test]
    fn test_pixel_get_and_set() {
        let mut buffer = vec![0u8; 16];
        let mut frame = VideoFrame::new(&mut buffer, 2, 2).expect("valid frame");

        assert!(frame.set_pixel(1, 1, PixelRgba::new(50, 100, 150, 200)));
        let px = frame.get_pixel(1, 1).expect("valid coordinate");
        assert_eq!(px, PixelRgba::new(50, 100, 150, 200));

        // Out of bounds checks
        assert!(!frame.set_pixel(2, 2, PixelRgba::new(1, 1, 1, 1)));
        assert!(frame.get_pixel(5, 5).is_none());
    }

    #[test]
    fn test_ffi_safe_memory_alloc_dealloc() {
        let ptr = ffi::guest_alloc(64);
        assert!(!ptr.is_null());
        ffi::guest_dealloc(ptr, 64);
    }

    #[test]
    fn test_panic_containment() {
        // Verify that internal guest panics do not escape, returning -5 (InternalPanic)
        let res = ffi::safe_exec(|| {
            panic!("Intentional test panic inside guest");
        });
        assert_eq!(res, Err(-5));
    }

    #[derive(Default)]
    struct GainEffect;

    impl AudioEffect for GainEffect {
        fn process_block(
            &mut self,
            block: &mut AudioBlock,
            params: &serde_json::Value,
        ) -> Result<(), SdkError> {
            let gain = params.get("gain").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            block.apply_gain(gain);
            Ok(())
        }
    }

    #[test]
    fn test_audio_effect_processing() {
        let mut ch_l = vec![0.5f32, -0.5f32];
        let mut ch_r = vec![1.0f32, -1.0f32];

        let mut block = AudioBlock::new(vec![&mut ch_l, &mut ch_r], 48000).expect("valid block");
        let mut effect = GainEffect::default();

        let params = serde_json::json!({ "gain": 0.5 });
        let res = effect.process_block(&mut block, &params);
        assert!(res.is_ok());

        assert_eq!(ch_l[0], 0.25);
        assert_eq!(ch_l[1], -0.25);
        assert_eq!(ch_r[0], 0.5);
        assert_eq!(ch_r[1], -0.5);
    }
}
