//! Software drawing into premultiplied 32-bit buffers, as `UpdateLayeredWindow` wants them.
//! Shapes are rasterized here; text is rendered by GDI into a scratch buffer and its
//! grayscale coverage is used as alpha, since GDI itself does not write alpha.

use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::*;

use super::wide;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub fn faded(self, factor: f32) -> Color {
        Color { a: (self.a as f32 * factor) as u8, ..self }
    }
}

/// Source-over blend of `color` at `coverage` (0..=1) onto a premultiplied 0xAARRGGBB pixel.
fn blend(dst: &mut u32, color: Color, coverage: f32) {
    let a = color.a as f32 / 255.0 * coverage.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    let mix = |shift: u32, src: u8| -> u32 {
        let d = ((*dst >> shift) & 0xFF) as f32;
        ((src as f32 * a + d * inv).round() as u32).min(255) << shift
    };
    let alpha = ((255.0 * a + ((*dst >> 24) & 0xFF) as f32 * inv).round() as u32).min(255) << 24;
    *dst = alpha | mix(16, color.r) | mix(8, color.g) | mix(0, color.b);
}

pub struct Canvas<'a> {
    pub px: &'a mut [u32],
    pub w: i32,
    pub h: i32,
}

impl Canvas<'_> {
    /// Anti-aliased rounded rectangle. Only the part left of `clip_right` is drawn,
    /// which is how a progress fill keeps the track's rounded shape.
    pub fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, clip_right: f32, color: Color) {
        let r = radius.min(w / 2.0).min(h / 2.0);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (hx, hy) = (w / 2.0 - r, h / 2.0 - r);
        let x0 = (x.floor() as i32).max(0);
        let x1 = ((x + w).ceil() as i32).min(self.w).min(clip_right.ceil() as i32);
        let y0 = (y.floor() as i32).max(0);
        let y1 = ((y + h).ceil() as i32).min(self.h);
        for py in y0..y1 {
            for px in x0..x1 {
                // Signed distance from the pixel centre to the rounded box.
                let qx = (px as f32 + 0.5 - cx).abs() - hx;
                let qy = (py as f32 + 0.5 - cy).abs() - hy;
                let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
                let distance = outside + qx.max(qy).min(0.0) - r;
                let coverage = (0.5 - distance).clamp(0.0, 1.0) * (clip_right - px as f32).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    blend(&mut self.px[(py * self.w + px) as usize], color, coverage);
                }
            }
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let d = radius * 2.0;
        self.round_rect(cx - radius, cy - radius, d, d, radius, f32::MAX, color);
    }
}

/// A top-down 32-bit DIB selected into a memory DC.
pub struct Surface {
    pub dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    pub w: i32,
    pub h: i32,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Option<Surface> {
        if w <= 0 || h <= 0 {
            return None;
        }
        unsafe {
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h; // negative: top-down rows
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;

            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return None;
            }
            let mut bits: *mut c_void = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            Some(Surface { dc, bitmap, previous, bits: bits as *mut u32, w, h })
        }
    }

    pub fn pixels(&mut self) -> &mut [u32] {
        // The DIB section owns w*h pixels for as long as `bitmap` lives.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }

    pub fn canvas(&mut self) -> Canvas<'_> {
        let (w, h) = (self.w, self.h);
        Canvas { px: self.pixels(), w, h }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

pub struct Font(HFONT);

impl Font {
    pub fn new(pixel_height: i32, bold: bool) -> Font {
        let face = wide("Segoe UI");
        let weight = if bold { 700 } else { 400 };
        // Grayscale anti-aliasing (quality 4): ClearType would leave colour fringes
        // once the coverage is turned into alpha.
        Font(unsafe { CreateFontW(-pixel_height, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 4, 0, face.as_ptr()) })
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}

#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Draws one vertically centred line of text inside `rect` onto `target`.
/// `scratch` must be the same size as `target`.
pub fn draw_text(target: &mut Surface, scratch: &mut Surface, font: &Font, text: &str, rect: RECT, align: Align, color: Color) {
    let text: Vec<u16> = text.encode_utf16().collect();
    let mut rect = rect;
    let align = match align {
        Align::Left => DT_LEFT,
        Align::Center => DT_CENTER,
        Align::Right => DT_RIGHT,
    };
    // Only the rows the line can touch are cleared and composited. The margin covers
    // glyphs that overshoot a rectangle slightly shorter than the font.
    let (width, height) = (target.w as usize, target.h);
    let first = (rect.top - 8).clamp(0, height) as usize;
    let last = (rect.bottom + 8).clamp(0, height) as usize;
    if first >= last {
        return;
    }
    let rows = first * width..last * width;
    scratch.pixels()[rows.clone()].fill(0);
    unsafe {
        let previous = SelectObject(scratch.dc, font.0);
        SetTextColor(scratch.dc, 0x00FF_FFFF);
        SetBkMode(scratch.dc, 1); // TRANSPARENT
        DrawTextW(
            scratch.dc,
            text.as_ptr(),
            text.len() as i32,
            &mut rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_NOCLIP | align,
        );
        SelectObject(scratch.dc, previous);
        GdiFlush();
    }
    // White on black: any channel is the glyph coverage.
    for (dst, src) in target.pixels()[rows.clone()].iter_mut().zip(scratch.pixels()[rows].iter()) {
        let coverage = (*src >> 8) & 0xFF;
        if coverage != 0 {
            blend(dst, color, coverage as f32 / 255.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(px: u32) -> u32 {
        px >> 24
    }

    #[test]
    fn blend_is_premultiplied() {
        let mut px = 0u32;
        blend(&mut px, Color::rgba(255, 255, 255, 128), 1.0);
        assert_eq!(px, 0x8080_8080);
        let mut px = 0u32;
        blend(&mut px, Color::rgb(10, 20, 30), 1.0);
        assert_eq!(px, 0xFF0A_141E);
    }

    #[test]
    fn round_rect_fills_inside_and_clips() {
        let mut buf = vec![0u32; 20 * 10];
        let mut canvas = Canvas { px: &mut buf, w: 20, h: 10 };
        canvas.round_rect(2.0, 2.0, 16.0, 6.0, 2.0, 10.0, Color::rgb(0, 255, 0));
        let at = |x: usize, y: usize| buf[y * 20 + x];
        assert_eq!(at(6, 5), 0xFF00_FF00); // inside
        assert_eq!(at(12, 5), 0); // right of the clip
        assert_eq!(at(0, 0), 0); // outside
        assert!(alpha(at(2, 2)) < 255); // rounded corner is only partly covered
    }
}
