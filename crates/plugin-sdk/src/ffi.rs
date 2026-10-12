//! Low-level FFI memory management and panic containment for WASM guests.

use std::alloc::{alloc, dealloc, Layout};
use std::panic::catch_unwind;

/// Safely allocates `size` bytes of linear memory for the host.
pub fn guest_alloc(size: usize) -> *mut u8 {
    if size == 0 {
        return std::ptr::null_mut();
    }
    match Layout::from_size_align(size, 8) {
        Ok(layout) => unsafe { alloc(layout) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// Safely frees `size` bytes previously allocated by `guest_alloc`.
pub fn guest_dealloc(ptr: *mut u8, size: usize) {
    if ptr.is_null() || size == 0 {
        return;
    }
    if let Ok(layout) = Layout::from_size_align(size, 8) {
        unsafe {
            dealloc(ptr, layout);
        }
    }
}

/// Packs pointer and length into a single 64-bit value for manifest transmission.
pub fn pack_ptr_len(ptr: u32, len: u32) -> u64 {
    ((ptr as u64) << 32) | (len as u64)
}

/// Executes a guest closure within `catch_unwind` to guarantee no unwinds escape the WASM boundary.
pub fn safe_exec<F, R>(f: F) -> Result<R, i32>
where
    F: FnOnce() -> Result<R, i32> + std::panic::UnwindSafe,
{
    match catch_unwind(f) {
        Ok(res) => res,
        Err(_) => Err(-5), // -5 corresponds to WasmStatusCode::InternalPanic
    }
}
