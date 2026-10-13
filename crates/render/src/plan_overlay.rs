//! Avoid preparing/uploading transparent borders of one-to-one ProRes-like overlays.

use std::sync::Arc;

use filmcraft_frame::{Chroma, PixelData, Region};
use filmcraft_geom::Affine;

use super::PlanLayer;
use crate::Blend;

/// Keep the original path for resampling, effects, managed colour, and destination-reading
/// blends. In the common one-to-one 4:4:4 case, a smaller source plus an integer translation
/// samples the same texels, with the same colour and alpha codes.
pub(super) fn trim(mut layer: PlanLayer) -> PlanLayer {
    let m = layer.matrix;
    if layer.fx.is_some()
        || layer.input.is_some()
        || layer.blend != Blend::Normal
        || m.a != 1.0
        || m.d != 1.0
        || m.b != 0.0
        || m.c != 0.0
        || !m.e.is_finite()
        || !m.f.is_finite()
        || m.e.fract() != 0.0
        || m.f.fract() != 0.0
        || m.e.abs() > 8_388_608.0
        || m.f.abs() > 8_388_608.0
    {
        return layer;
    }
    let f = &layer.frame;
    let (w, h) = (f.width as usize, f.height as usize);
    let Some(px) = w.checked_mul(h).filter(|n| *n > 0) else { return layer };
    // Do not change chroma siting or turn malformed planes into apparently valid pictures.
    if !matches!(&f.data,
        PixelData::Yuv16 { planes, chroma: Chroma::C444, bits: 9..=16, alpha: Some(a) }
            if a.len() == px && planes.iter().all(|p| p.len() == px))
    {
        return layer;
    }
    let Some(r) = f.alpha_region(1) else { return layer };
    // Retain a transparent texel around the visible rectangle for the shader's bilinear loads.
    // A fully transparent frame becomes one transparent texel, preserving the layer's order.
    let r = if r.is_empty() {
        Region { x: 0, y: 0, w: 1, h: 1 }
    } else {
        let (x, y) = (r.x.saturating_sub(1), r.y.saturating_sub(1));
        Region { x, y, w: r.x.saturating_add(r.w).saturating_add(1).min(w) - x, h: r.y.saturating_add(r.h).saturating_add(1).min(h) - y }
    };
    // A copy only pays for itself when it saves a substantial part of the upload.
    if r.w.saturating_mul(r.h) > px / 2 {
        return layer;
    }
    layer.frame = Arc::new(f.cropped(r));
    layer.matrix = m.then_apply(&Affine::translate(r.x as f64, r.y as f64));
    layer
}

#[cfg(test)]
mod tests {
    use super::super::{FramePlan, execute_cpu};
    use super::*;
    use filmcraft_frame::VideoFrame;

    fn layer() -> PlanLayer {
        let (w, h) = (31, 23);
        let alpha = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if (5..21).contains(&x) && (3..7).contains(&y) { if i % 2 == 0 { 2048 } else { 4095 } } else { 0 }
            })
            .collect();
        PlanLayer::new(
            Arc::new(VideoFrame {
                width: w as u32,
                height: h as u32,
                data: PixelData::Yuv16 {
                    planes: [Arc::new(vec![1000; w * h]), Arc::new(vec![2000; w * h]), Arc::new(vec![2200; w * h])],
                    chroma: Chroma::C444,
                    bits: 12,
                    alpha: Some(Arc::new(alpha)),
                },
                color: filmcraft_color::ColorInfo::REC709,
                par: (1, 1),
                pts: Default::default(),
            }),
            Affine::IDENTITY,
            1.0,
            Blend::Normal,
        )
    }

    #[test]
    fn trimming_preserves_the_reference_picture_and_metadata() {
        for (x, y) in [(0, 0), (3, 4), (-6, -4), (100, 100)] {
            for opacity in [0.0, 0.4, 1.0] {
                let mut l = layer();
                l.matrix = Affine::translate(x as f64, y as f64);
                l.opacity = opacity;
                let small = trim(l.clone());
                assert_eq!((small.frame.width, small.frame.height), (18, 6));
                assert_eq!(small.matrix, Affine::translate((x + 4) as f64, (y + 2) as f64));
                assert_eq!(small.frame.color, l.frame.color);
                assert_eq!(small.frame.pts, l.frame.pts);
                assert_eq!(small.frame.par, l.frame.par);
                let render = |layer| execute_cpu(&FramePlan::Layers { width: 31, height: 23, layers: vec![layer] }).px;
                let (got, expected) = (render(small), render(l));
                let difference = got.iter().zip(&expected).position(|(a, b)| a.to_bits() != b.to_bits());
                assert_eq!(difference, None, "translation ({x},{y}), opacity {opacity}");
            }
        }
    }

    #[test]
    fn unsafe_or_unprofitable_cases_keep_the_original_allocation() {
        let base = layer();
        let unchanged = |l: PlanLayer| assert!(Arc::ptr_eq(&l.frame, &trim(l.clone()).frame));
        for matrix in [
            Affine::scale(0.5, 0.5),
            Affine::rotate_deg(90.0),
            Affine::translate(0.5, 0.0),
            Affine::translate(f64::NAN, 0.0),
            Affine::translate(9_000_000.0, 0.0),
        ] {
            unchanged(PlanLayer { matrix, ..base.clone() });
        }
        unchanged(PlanLayer { blend: Blend::Multiply, ..base.clone() });
        unchanged(PlanLayer { fx: Some(Arc::new(super::super::LayerFx { size: (31, 23), decimation: 1, ops: vec![] })), ..base.clone() });
        for case in 0..6 {
            let mut f = (*base.frame).clone();
            if let PixelData::Yuv16 { planes, chroma, bits, alpha } = &mut f.data {
                match case {
                    0 => *chroma = Chroma::C422,
                    1 => *bits = 17,
                    2 => *alpha = None,
                    3 => *alpha = Some(Arc::new(vec![4095; 31 * 23])),
                    4 => *alpha = Some(Arc::new(vec![0; 2])),
                    _ => planes[0] = Arc::new(vec![0; 2]),
                }
            }
            unchanged(PlanLayer { frame: Arc::new(f), ..base.clone() });
        }
    }
}
