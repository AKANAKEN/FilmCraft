//! High-level, safe video frame abstraction for plugin developers.
//!
//! Provides bound-checked, zero-panic pixel manipulation over raw RGBA8 buffers.

/// A single 32-bit RGBA pixel.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PixelRgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl PixelRgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// A safe wrapper representing an uncompressed RGBA8 video frame buffer.
pub struct VideoFrame<'a> {
    data: &'a mut [u8],
    width: u32,
    height: u32,
}

impl<'a> VideoFrame<'a> {
    /// Wraps a raw mutable byte slice with given dimensions.
    /// Returns None if buffer length is insufficient for `width * height * 4`.
    pub fn new(data: &'a mut [u8], width: u32, height: u32) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;

        if data.len() < expected {
            None
        } else {
            Some(Self { data, width, height })
        }
    }

    #[inline(always)]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[inline(always)]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[inline(always)]
    pub fn raw_slice(&self) -> &[u8] {
        self.data
    }

    #[inline(always)]
    pub fn raw_slice_mut(&mut self) -> &mut [u8] {
        self.data
    }

    /// Read pixel at `(x, y)` with boundary checking.
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<PixelRgba> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        Some(PixelRgba {
            r: self.data.get(offset).copied().unwrap_or(0),
            g: self.data.get(offset + 1).copied().unwrap_or(0),
            b: self.data.get(offset + 2).copied().unwrap_or(0),
            a: self.data.get(offset + 3).copied().unwrap_or(255),
        })
    }

    /// Overwrite pixel at `(x, y)` with boundary checking.
    pub fn set_pixel(&mut self, x: u32, y: u32, pixel: PixelRgba) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let offset = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        if let Some(chunk) = self.data.get_mut(offset..offset + 4) {
            chunk[0] = pixel.r;
            chunk[1] = pixel.g;
            chunk[2] = pixel.b;
            chunk[3] = pixel.a;
            true
        } else {
            false
        }
    }

    /// Iterates over every pixel in place with maximum performance and zero panic risk.
    pub fn for_each_pixel_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut PixelRgba),
    {
        for chunk in self.data.chunks_exact_mut(4) {
            let mut px = PixelRgba {
                r: chunk[0],
                g: chunk[1],
                b: chunk[2],
                a: chunk[3],
            };
            f(&mut px);
            chunk[0] = px.r;
            chunk[1] = px.g;
            chunk[2] = px.b;
            chunk[3] = px.a;
        }
    }
}
