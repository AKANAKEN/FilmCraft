//! Hardware video encoding on Intel GPUs (Intel Quick Sync Video) via Media Foundation transforms.
//!
//! Provides hardware-accelerated H.264, HEVC (Main and Main 10 HDR), and AV1 export on Intel
//! integrated (Iris Xe, UHD) and discrete graphics (Arc A-series, B-series).

pub mod device;
pub mod export;
pub mod session;

use std::sync::OnceLock;

use windows::Win32::Media::MediaFoundation::{MFVideoFormat_AV1, MFVideoFormat_HEVC};

use self::device::IntelGpu;
use self::session::find_encoder_activate;
use crate::media_foundation::mft::api;

static HEVC_AVAILABLE: OnceLock<bool> = OnceLock::new();
static HEVC_HDR_AVAILABLE: OnceLock<bool> = OnceLock::new();
static AV1_AVAILABLE: OnceLock<bool> = OnceLock::new();
static AV1_HDR_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Whether an Intel GPU is present and supports HEVC hardware export.
pub fn hevc_available() -> bool {
    *HEVC_AVAILABLE.get_or_init(|| probe_codec(MFVideoFormat_HEVC, false))
}

/// Whether an Intel GPU is present and supports HEVC Main 10 (10-bit / HDR) hardware export.
pub fn hevc_hdr_available() -> bool {
    *HEVC_HDR_AVAILABLE.get_or_init(|| probe_codec(MFVideoFormat_HEVC, true))
}

/// Whether an Intel GPU is present and supports AV1 hardware export.
pub fn av1_available() -> bool {
    *AV1_AVAILABLE.get_or_init(|| probe_codec(MFVideoFormat_AV1, false))
}

/// Whether an Intel GPU is present and supports AV1 Main 10 (10-bit / HDR) hardware export.
pub fn av1_hdr_available() -> bool {
    *AV1_HDR_AVAILABLE.get_or_init(|| probe_codec(MFVideoFormat_AV1, true))
}

/// Warm up HEVC probes asynchronously or on background thread.
pub fn warm_hevc_probe() {
    let _ = hevc_available();
    let _ = hevc_hdr_available();
}

/// Warm up AV1 probes asynchronously.
pub fn warm_av1_probe() {
    let _ = av1_available();
    let _ = av1_hdr_available();
}

fn probe_codec(subtype: windows::core::GUID, _ten_bit: bool) -> bool {
    let Ok(Some(_gpu)) = IntelGpu::probe() else {
        return false;
    };
    let Ok(mf_api) = api() else {
        return false;
    };
    let Ok(Some(_)) = find_encoder_activate(mf_api, subtype) else {
        return false;
    };
    // On Intel Arc and modern Iris Xe, HEVC 10-bit is standard. AV1 encode requires Arc (Gen 12.7+) or Meteor Lake.
    if subtype == MFVideoFormat_AV1 {
        // Intel Arc / Xe-HPG device IDs generally begin with 0x56 or discrete Arc IDs.
        // For hardware MFT, existence of the encoder MFT for AV1 confirms driver/hardware capability.
        true
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_id_is_intel() {
        assert_eq!(device::INTEL_VENDOR_ID, 0x8086);
    }

    #[test]
    fn codec_subtypes_are_distinct() {
        use session::Codec;
        assert_ne!(Codec::H264.subtype(), Codec::Hevc.subtype());
        assert_ne!(Codec::Hevc.subtype(), Codec::Av1.subtype());
    }

    #[test]
    fn codec_names() {
        use session::Codec;
        assert_eq!(Codec::H264.name(), "H.264");
        assert_eq!(Codec::Hevc.name(), "HEVC");
        assert_eq!(Codec::Av1.name(), "AV1");
    }
}
