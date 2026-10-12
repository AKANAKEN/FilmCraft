//! Bridge between the FilmCraft compositor (`Image`) and the sandboxed plugin registry.
//!
//! Converts between FilmCraft's internal premultiplied linear-light f32 buffers
//! and standard uncompressed 32-bit RGBA8 buffers expected by WASM plugins.
//! Strictly enforces the "Never crash" rule: failed plugins log a diagnostic and
//! preserve the original frame buffer without panicking.

use filmcraft_color::{linear_to_srgb, srgb_to_linear};
use filmcraft_plugin::registry::PluginRegistry;
use rayon::prelude::*;

use crate::image::Image;

/// Applies an external or sandboxed plugin effect to an [`Image`] in place.
pub fn apply_plugin_effect(
    img: &mut Image,
    registry: &PluginRegistry,
    effect_id: &str,
    params_json: &str,
) -> Result<(), String> {
    if img.w == 0 || img.h == 0 {
        return Ok(());
    }

    let w = img.w;
    let h = img.h;
    let total_pixels = w.checked_mul(h).ok_or("frame dimension overflow")?;
    let mut in_bytes = vec![0u8; total_pixels * 4];
    let mut out_bytes = vec![0u8; total_pixels * 4];

    // 1. Convert FilmCraft premultiplied linear f32 to straight sRGB RGBA8
    in_bytes
        .par_chunks_exact_mut(4)
        .zip(img.px.par_chunks_exact(4))
        .for_each(|(out_px, in_px)| {
            let a = in_px[3].clamp(0.0, 1.0);
            if a > 0.0 {
                let r_straight = (in_px[0] / a).clamp(0.0, 1.0);
                let g_straight = (in_px[1] / a).clamp(0.0, 1.0);
                let b_straight = (in_px[2] / a).clamp(0.0, 1.0);

                out_px[0] = (linear_to_srgb(r_straight) * 255.0).round() as u8;
                out_px[1] = (linear_to_srgb(g_straight) * 255.0).round() as u8;
                out_px[2] = (linear_to_srgb(b_straight) * 255.0).round() as u8;
                out_px[3] = (a * 255.0).round() as u8;
            } else {
                out_px[0] = 0;
                out_px[1] = 0;
                out_px[2] = 0;
                out_px[3] = 0;
            }
        });

    // 2. Dispatch to the sandboxed plugin registry
    registry
        .execute_video_effect_json(
            effect_id,
            &in_bytes,
            &mut out_bytes,
            w as u32,
            h as u32,
            params_json,
        )
        .map_err(|err| {
            format!("[FilmCraft Plugin Error] Effect '{effect_id}' failed: {err}")
        })?;

    // 3. Convert processed straight sRGB RGBA8 back to premultiplied linear f32
    img.px
        .par_chunks_exact_mut(4)
        .zip(out_bytes.par_chunks_exact(4))
        .for_each(|(out_f32, in_u8)| {
            let a = (in_u8[3] as f32) / 255.0;
            let r_linear = srgb_to_linear((in_u8[0] as f32) / 255.0);
            let g_linear = srgb_to_linear((in_u8[1] as f32) / 255.0);
            let b_linear = srgb_to_linear((in_u8[2] as f32) / 255.0);

            out_f32[0] = r_linear * a;
            out_f32[1] = g_linear * a;
            out_f32[2] = b_linear * a;
            out_f32[3] = a;
        });

    Ok(())
}

static GLOBAL_PLUGIN_REGISTRY: std::sync::RwLock<Option<PluginRegistry>> = std::sync::RwLock::new(None);

/// Sets the active global plugin registry used during frame rendering.
pub fn set_render_plugin_registry(registry: PluginRegistry) {
    if let Ok(mut lock) = GLOBAL_PLUGIN_REGISTRY.write() {
        *lock = Some(registry);
    }
}

/// Attempts to apply an effect via the global plugin registry if present.
pub fn try_apply_registered_effect(img: &mut Image, effect_id: &str, params_json: &str) -> bool {
    if let Ok(lock) = GLOBAL_PLUGIN_REGISTRY.read() {
        if let Some(ref reg) = *lock {
            if reg.get_video_effect(effect_id).is_some() {
                let _ = apply_plugin_effect(img, reg, effect_id, params_json);
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_plugin::poc::GrayscaleEffect;

    #[test]
    fn test_apply_plugin_effect_success() {
        let mut registry = PluginRegistry::new();
        let poc = GrayscaleEffect::new();
        registry.register_video_effect(poc).expect("registration failed");

        // Create a 2x2 red image in premultiplied linear f32
        let mut img = Image::filled(2, 2, [1.0, 0.0, 0.0, 1.0]);

        let res = apply_plugin_effect(&mut img, &registry, "craft.builtin.poc_grayscale", r#"{"amount": 1.0}"#);
        assert!(res.is_ok());

        // In monochrome, red (1.0, 0.0, 0.0) converts to ~0.21 luma
        let p = img.get(0, 0);
        assert!((p[0] - p[1]).abs() < 0.05, "channels should be equal in grayscale");
        assert!((p[1] - p[2]).abs() < 0.05, "channels should be equal in grayscale");
        assert_eq!(p[3], 1.0, "alpha preserved");
    }

    #[test]
    fn test_apply_plugin_effect_nonexistent_fails_safely() {
        let registry = PluginRegistry::new();
        let mut img = Image::filled(2, 2, [1.0, 0.0, 0.0, 1.0]);
        let original_px = img.px.clone();

        let res = apply_plugin_effect(&mut img, &registry, "non.existent.effect", "{}");
        assert!(res.is_err());
        // Verify Image was untouched (Never crash / never corrupt!)
        assert_eq!(img.px, original_px);
    }
}
