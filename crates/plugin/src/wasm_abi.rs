//! FilmCraft WebAssembly Application Binary Interface (ABI) definitions.
//!
//! Standardized C-compatible symbols exported by FilmCraft `.wasm` plugins.
//! Designed for high performance, zero panic propagation, and clean memory boundaries.

/// Function name exported by WASM module to allocate memory within guest linear memory.
/// Signature: `(size: u32) -> u32` (returns memory pointer)
pub const ALLOC_SYMBOL: &str = "filmcraft_alloc";

/// Function name exported by WASM module to free memory within guest linear memory.
/// Signature: `(ptr: u32, size: u32) -> ()`
pub const DEALLOC_SYMBOL: &str = "filmcraft_dealloc";

/// Function name exported by WASM module to return its JSON manifest.
/// Signature: `() -> u64` (packed: high 32 bits = ptr, low 32 bits = len)
pub const GET_MANIFEST_SYMBOL: &str = "filmcraft_get_manifest";

/// Function name exported by video effect plugins to process a single RGBA8 frame.
/// Signature: `(in_ptr: u32, out_ptr: u32, width: u32, height: u32, params_ptr: u32, params_len: u32) -> i32`
/// Returns: `0` on success, or a negative integer corresponding to an error code.
pub const PROCESS_VIDEO_FRAME_SYMBOL: &str = "filmcraft_process_video_frame";

/// Error codes returned across the WASM boundary.
#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WasmStatusCode {
    Ok = 0,
    InvalidDimensions = -1,
    BufferSizeMismatch = -2,
    InvalidParameter = -3,
    ExecutionFailed = -4,
    InternalPanic = -5,
}

impl WasmStatusCode {
    pub fn from_i32(code: i32) -> Self {
        match code {
            0 => Self::Ok,
            -1 => Self::InvalidDimensions,
            -2 => Self::BufferSizeMismatch,
            -3 => Self::InvalidParameter,
            -4 => Self::ExecutionFailed,
            _ => Self::InternalPanic,
        }
    }
}

/// Utility to pack a 32-bit pointer and 32-bit length into a 64-bit integer.
pub fn pack_ptr_len(ptr: u32, len: u32) -> u64 {
    ((ptr as u64) << 32) | (len as u64)
}

/// Utility to unpack a 64-bit integer into a 32-bit pointer and 32-bit length.
pub fn unpack_ptr_len(packed: u64) -> (u32, u32) {
    let ptr = (packed >> 32) as u32;
    let len = (packed & 0xFFFF_FFFF) as u32;
    (ptr, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ptr_len_packing() {
        let ptr = 0x1234_5678;
        let len = 0x000F_FFFF;
        let packed = pack_ptr_len(ptr, len);
        let (unpacked_ptr, unpacked_len) = unpack_ptr_len(packed);
        assert_eq!(unpacked_ptr, ptr);
        assert_eq!(unpacked_len, len);
    }
}
