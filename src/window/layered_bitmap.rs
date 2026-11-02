//! Scoped GDI resources for layered-window painting.
//!
//! A DIB must be reselected out of its memory DC before deletion, and a
//! window DC must be returned to the same HWND. These guards handle both
//! normal painting and early exits without leaving selected GDI objects.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};

pub(super) struct WindowDc {
    owner: HWND,
    dc: HDC,
}

impl WindowDc {
    pub(super) fn acquire(owner: HWND) -> Option<Self> {
        let dc = unsafe { GetDC(owner) };
        (!dc.0.is_null()).then_some(Self { owner, dc })
    }

    pub(super) fn handle(&self) -> HDC {
        self.dc
    }
}

impl Drop for WindowDc {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseDC(self.owner, self.dc);
        }
    }
}

/// Owns a 32-bit, top-down DIB selected into its memory DC. The raw pixel
/// slice remains valid until this surface is dropped.
pub(super) struct DibSurface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    pixel_count: usize,
}

impl DibSurface {
    pub(super) fn create(reference: HDC, width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let pixel_count = (width as usize).checked_mul(height as usize)?;
        let negative_height = height.checked_neg()?;
        unsafe {
            let dc = CreateCompatibleDC(reference);
            if dc.0.is_null() {
                return None;
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: negative_height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                dc, &info, DIB_RGB_COLORS, &mut pixels, None, 0,
            ).unwrap_or_default();
            if bitmap.is_invalid() || pixels.is_null() {
                if !bitmap.is_invalid() {
                    let _ = DeleteObject(bitmap);
                }
                let _ = DeleteDC(dc);
                return None;
            }

            let previous = SelectObject(dc, bitmap);
            if previous.0.is_null() || previous.0 as isize == -1 {
                let _ = DeleteObject(bitmap);
                let _ = DeleteDC(dc);
                return None;
            }

            Some(Self {
                dc, bitmap, previous,
                bits: pixels.cast::<u32>(),
                pixel_count,
            })
        }
    }

    pub(super) fn handle(&self) -> HDC {
        self.dc
    }

    pub(super) fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: CreateDIBSection returned a non-null 32-bit pixel buffer
        // of checked size, owned by this surface for the entire borrow.
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.pixel_count) }
    }
}

impl Drop for DibSurface {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_dimensions_without_allocating_gdi_resources() {
        let null_dc = HDC::default();
        assert!(DibSurface::create(null_dc, 0, 32).is_none());
        assert!(DibSurface::create(null_dc, 32, 0).is_none());
        assert!(DibSurface::create(null_dc, -1, 32).is_none());
        assert!(DibSurface::create(null_dc, 32, -1).is_none());
    }
}
