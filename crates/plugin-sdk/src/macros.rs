//! Declarative export macro for FilmCraft WASM plugins.

/// Exports all required C-ABI symbols (`filmcraft_alloc`, `filmcraft_dealloc`,
/// `filmcraft_get_manifest`, `filmcraft_process_video_frame`) for a [`crate::VideoEffect`].
#[macro_export]
macro_rules! export_video_effect {
    ($effect_ty:ty, $manifest_json:expr) => {
        #[no_mangle]
        pub extern "C" fn filmcraft_alloc(size: u32) -> u32 {
            $crate::ffi::guest_alloc(size as usize) as u32
        }

        #[no_mangle]
        pub extern "C" fn filmcraft_dealloc(ptr: u32, size: u32) {
            $crate::ffi::guest_dealloc(ptr as *mut u8, size as usize);
        }

        #[no_mangle]
        pub extern "C" fn filmcraft_get_manifest() -> u64 {
            let json = $manifest_json;
            let bytes = json.as_bytes();
            let ptr = $crate::ffi::guest_alloc(bytes.len()) as *mut u8;
            if ptr.is_null() {
                return 0;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            }
            $crate::ffi::pack_ptr_len(ptr as u32, bytes.len() as u32)
        }

        #[no_mangle]
        pub extern "C" fn filmcraft_process_video_frame(
            in_ptr: u32,
            out_ptr: u32,
            width: u32,
            height: u32,
            params_ptr: u32,
            params_len: u32,
        ) -> i32 {
            $crate::ffi::safe_exec(|| {
                if in_ptr == 0 || out_ptr == 0 || width == 0 || height == 0 {
                    return Err(-1); // InvalidDimensions
                }

                let num_pixels = (width as usize).checked_mul(height as usize).ok_or(-1)?;
                let byte_len = num_pixels.checked_mul(4).ok_or(-1)?;

                let in_slice = unsafe { std::slice::from_raw_parts(in_ptr as *const u8, byte_len) };
                let out_slice = unsafe { std::slice::from_raw_parts_mut(out_ptr as *mut u8, byte_len) };

                // Copy input to output buffer initially
                out_slice.copy_from_slice(in_slice);

                let params: serde_json::Value = if params_ptr != 0 && params_len > 0 {
                    let p_slice = unsafe { std::slice::from_raw_parts(params_ptr as *const u8, params_len as usize) };
                    serde_json::from_slice(p_slice).unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Null
                };

                let mut frame = match $crate::VideoFrame::new(out_slice, width, height) {
                    Some(f) => f,
                    None => return Err(-2), // BufferSizeMismatch
                };

                let mut effect = <$effect_ty>::default();
                match <$effect_ty as $crate::VideoEffect>::process_frame(&mut effect, &mut frame, &params) {
                    Ok(()) => Ok(0),
                    Err(_) => Err(-4), // ExecutionFailed
                }
            })
            .unwrap_or_else(|err_code| err_code)
        }
    };
}

/// Exports all required C-ABI symbols (`filmcraft_alloc`, `filmcraft_dealloc`,
/// `filmcraft_get_manifest`, `filmcraft_process_audio_block`) for an [`crate::AudioEffect`].
#[macro_export]
macro_rules! export_audio_effect {
    ($effect_ty:ty, $manifest_json:expr) => {
        #[no_mangle]
        pub extern "C" fn filmcraft_alloc(size: u32) -> u32 {
            $crate::ffi::guest_alloc(size as usize) as u32
        }

        #[no_mangle]
        pub extern "C" fn filmcraft_dealloc(ptr: u32, size: u32) {
            $crate::ffi::guest_dealloc(ptr as *mut u8, size as usize);
        }

        #[no_mangle]
        pub extern "C" fn filmcraft_get_manifest() -> u64 {
            let json = $manifest_json;
            let bytes = json.as_bytes();
            let ptr = $crate::ffi::guest_alloc(bytes.len()) as *mut u8;
            if ptr.is_null() {
                return 0;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            }
            $crate::ffi::pack_ptr_len(ptr as u32, bytes.len() as u32)
        }

        #[no_mangle]
        pub extern "C" fn filmcraft_process_audio_block(
            data_ptr: u32,
            num_channels: u32,
            num_samples: u32,
            sample_rate: u32,
            params_ptr: u32,
            params_len: u32,
        ) -> i32 {
            $crate::ffi::safe_exec(|| {
                if data_ptr == 0 || num_channels == 0 || num_samples == 0 || sample_rate == 0 {
                    return Err(-1); // InvalidDimensions / invalid args
                }

                let total_floats = (num_channels as usize).checked_mul(num_samples as usize).ok_or(-1)?;
                let byte_len = total_floats.checked_mul(std::mem::size_of::<f32>()).ok_or(-1)?;

                let floats_slice = unsafe {
                    std::slice::from_raw_parts_mut(data_ptr as *mut f32, total_floats)
                };

                let channels_vec: Vec<&mut [f32]> = floats_slice
                    .chunks_exact_mut(num_samples as usize)
                    .collect();

                let params: serde_json::Value = if params_ptr != 0 && params_len > 0 {
                    let p_slice = unsafe { std::slice::from_raw_parts(params_ptr as *const u8, params_len as usize) };
                    serde_json::from_slice(p_slice).unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Null
                };

                let mut block = match $crate::audio_block::AudioBlock::new(channels_vec, sample_rate) {
                    Some(b) => b,
                    None => return Err(-2),
                };

                let mut effect = <$effect_ty>::default();
                match <$effect_ty as $crate::AudioEffect>::process_block(&mut effect, &mut block, &params) {
                    Ok(()) => Ok(0),
                    Err(_) => Err(-4), // ExecutionFailed
                }
            })
            .unwrap_or_else(|err_code| err_code)
        }
    };
}
