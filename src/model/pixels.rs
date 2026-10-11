//! Pixel helpers shared by the model, tools, render and export layers.
//!
//! Pixmaps are premultiplied RGBA8 (tiny-skia's native format). PNG/GIF/egui interop needs
//! straight alpha in places, so the conversions live here, with rounding chosen so that
//! premultiplied -> straight -> premultiplied is lossless (tested exhaustively).

use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

/// Integer rectangle, half-open: covers `x0..x1` by `y0..y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn full(w: u32, h: u32) -> Self {
        Self::new(0, 0, w as i32, h as i32)
    }

    pub fn width(&self) -> u32 {
        (self.x1 - self.x0).max(0) as u32
    }

    pub fn height(&self) -> u32 {
        (self.y1 - self.y0).max(0) as u32
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn union(&self, o: &IRect) -> IRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        IRect::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }

    pub fn intersect(&self, o: &IRect) -> IRect {
        IRect::new(
            self.x0.max(o.x0),
            self.y0.max(o.y0),
            self.x1.min(o.x1),
            self.y1.min(o.y1),
        )
    }

    pub fn inflate(&self, by: i32) -> IRect {
        IRect::new(self.x0 - by, self.y0 - by, self.x1 + by, self.y1 + by)
    }

    pub fn clamp_to(&self, w: u32, h: u32) -> IRect {
        self.intersect(&IRect::full(w, h))
    }

    pub fn area_bytes(&self) -> usize {
        self.width() as usize * self.height() as usize * 4
    }
}

#[inline]
pub fn premul_u8(c: u8, a: u8) -> u8 {
    ((c as u32 * a as u32 + 127) / 255) as u8
}

#[inline]
pub fn demul_u8(c: u8, a: u8) -> u8 {
    if a == 0 {
        0
    } else {
        (((c as u32 * 255) + a as u32 / 2) / a as u32).min(255) as u8
    }
}

/// Copies the pixels of `r` (must lie inside the pixmap) into a tightly packed buffer.
pub fn extract_rect(pm: &Pixmap, r: IRect) -> Vec<u8> {
    let r = r.clamp_to(pm.width(), pm.height());
    let (w, h) = (r.width() as usize, r.height() as usize);
    let stride = pm.width() as usize * 4;
    let data = pm.data();
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let start = (r.y0 as usize + y) * stride + r.x0 as usize * 4;
        out.extend_from_slice(&data[start..start + w * 4]);
    }
    out
}

/// Inverse of [`extract_rect`].
pub fn write_rect(pm: &mut Pixmap, r: IRect, src: &[u8]) {
    let r = r.clamp_to(pm.width(), pm.height());
    let (w, h) = (r.width() as usize, r.height() as usize);
    if src.len() < w * h * 4 {
        return;
    }
    let stride = pm.width() as usize * 4;
    let data = pm.data_mut();
    for y in 0..h {
        let start = (r.y0 as usize + y) * stride + r.x0 as usize * 4;
        data[start..start + w * 4].copy_from_slice(&src[y * w * 4..(y + 1) * w * 4]);
    }
}

/// Straight-alpha RGBA bytes from a premultiplied pixmap.
pub fn pixmap_to_straight(pm: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(pm.data().len());
    for px in pm.data().chunks_exact(4) {
        let a = px[3];
        out.extend_from_slice(&[
            demul_u8(px[0], a),
            demul_u8(px[1], a),
            demul_u8(px[2], a),
            a,
        ]);
    }
    out
}

/// Builds a pixmap from straight-alpha RGBA bytes. Returns `None` on a size mismatch.
pub fn pixmap_from_straight(w: u32, h: u32, rgba: &[u8]) -> Option<Pixmap> {
    if rgba.len() != w as usize * h as usize * 4 {
        return None;
    }
    let mut pm = Pixmap::new(w, h)?;
    for (dst, src) in pm.data_mut().chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
        let a = src[3];
        dst.copy_from_slice(&[
            premul_u8(src[0], a),
            premul_u8(src[1], a),
            premul_u8(src[2], a),
            a,
        ]);
    }
    Some(pm)
}

/// Composites `pm` over an opaque colour and returns tightly packed straight RGBA (alpha 255).
pub fn flatten_over(pm: Option<&Pixmap>, w: u32, h: u32, bg: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    match pm {
        Some(pm) => {
            for px in pm.data().chunks_exact(4) {
                let inv = 255 - px[3] as u32;
                let blend = |s: u8, d: u8| (s as u32 + (d as u32 * inv + 127) / 255).min(255) as u8;
                out.extend_from_slice(&[
                    blend(px[0], bg[0]),
                    blend(px[1], bg[1]),
                    blend(px[2], bg[2]),
                    255,
                ]);
            }
        }
        None => {
            for _ in 0..(w as usize * h as usize) {
                out.extend_from_slice(&[bg[0], bg[1], bg[2], 255]);
            }
        }
    }
    out
}

/// 2x2 box filter in premultiplied space (exact average, so no halos). Dimensions floor to /2.
pub fn downscale_half(src: &Pixmap) -> Pixmap {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (dw, dh) = ((sw / 2).max(1), (sh / 2).max(1));
    let mut dst = Pixmap::new(dw as u32, dh as u32).expect("non-zero size");
    let s = src.data();
    let d = dst.data_mut();
    for y in 0..dh {
        let y0 = (y * 2).min(sh - 1);
        let y1 = (y * 2 + 1).min(sh - 1);
        for x in 0..dw {
            let x0 = (x * 2).min(sw - 1);
            let x1 = (x * 2 + 1).min(sw - 1);
            for c in 0..4 {
                let sum = s[(y0 * sw + x0) * 4 + c] as u32
                    + s[(y0 * sw + x1) * 4 + c] as u32
                    + s[(y1 * sw + x0) * 4 + c] as u32
                    + s[(y1 * sw + x1) * 4 + c] as u32;
                d[(y * dw + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    dst
}

/// Scales `src` to fit inside `w`x`h` preserving aspect ratio, centred ("contain"), on a
/// transparent canvas. Large reductions pre-shrink with a box filter to avoid aliasing.
pub fn fit_contain(src: &Pixmap, w: u32, h: u32) -> Option<Pixmap> {
    let mut out = Pixmap::new(w, h)?;
    let mut owned: Option<Pixmap> = None;
    loop {
        let c = owned.as_ref().unwrap_or(src);
        if c.width() >= w * 2 && c.height() >= h * 2 {
            owned = Some(downscale_half(c));
        } else {
            break;
        }
    }
    let cur = owned.as_ref().unwrap_or(src);
    let scale = (w as f32 / cur.width() as f32).min(h as f32 / cur.height() as f32);
    let dw = cur.width() as f32 * scale;
    let dh = cur.height() as f32 * scale;
    let tx = (w as f32 - dw) / 2.0;
    let ty = (h as f32 - dh) / 2.0;
    let quality = if (scale - 1.0).abs() < 1e-4 {
        FilterQuality::Nearest
    } else {
        FilterQuality::Bicubic
    };
    out.draw_pixmap(
        0,
        0,
        cur.as_ref(),
        &PixmapPaint {
            quality,
            ..PixmapPaint::default()
        },
        Transform::from_row(scale, 0.0, 0.0, scale, tx, ty),
        None,
    );
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiply_roundtrip_is_lossless_for_all_valid_pixels() {
        for a in 0..=255u16 {
            for c in 0..=a {
                let d = demul_u8(c as u8, a as u8);
                let back = premul_u8(d, a as u8);
                assert_eq!(back as u16, c, "a={a} c={c} d={d}");
            }
        }
    }

    #[test]
    fn rect_extract_write_roundtrip() {
        let mut pm = Pixmap::new(10, 8).unwrap();
        for (i, b) in pm.data_mut().iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let r = IRect::new(2, 1, 7, 5);
        let part = extract_rect(&pm, r);
        assert_eq!(part.len(), r.area_bytes());
        let mut other = Pixmap::new(10, 8).unwrap();
        write_rect(&mut other, r, &part);
        assert_eq!(extract_rect(&other, r), part);
    }

    #[test]
    fn downscale_half_averages() {
        let mut pm = Pixmap::new(2, 2).unwrap();
        pm.data_mut()
            .copy_from_slice(&[0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255]);
        let h = downscale_half(&pm);
        assert_eq!(h.data(), &[128, 0, 0, 128]);
    }

    #[test]
    fn fit_contain_centres_wide_image() {
        let mut src = Pixmap::new(4, 2).unwrap();
        src.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        let out = fit_contain(&src, 8, 8).unwrap();
        // Scaled 2x -> 8x4, centred vertically: rows 2..6 filled, rows 0..2 empty.
        assert_eq!(out.pixel(4, 0).unwrap().alpha(), 0);
        assert_eq!(out.pixel(4, 3).unwrap().alpha(), 255);
        assert_eq!(out.pixel(4, 7).unwrap().alpha(), 0);
    }
}
