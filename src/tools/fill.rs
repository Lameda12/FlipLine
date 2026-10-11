//! Flood fill: 4-connected scanline fill with colour tolerance that "tucks under" anti-aliased
//! edges. The region is grown a couple of pixels up the AA ramp of neighbouring ink and the fill
//! colour is painted *behind* existing pixels there, so line art stays on top and no light halo
//! is left between ink and fill.

use tiny_skia::Pixmap;

use super::{Sample, Tool, ToolId, ToolOutput, ToolSettings};
use crate::history::PixelEdit;
use crate::model::pixels::{extract_rect, IRect};
use crate::model::Frame;

const IN_REGION: u8 = 1;
const GROWN: u8 = 2;

pub struct Region {
    mask: Vec<u8>,
    bounds: IRect,
    seed: [u8; 4],
}

impl Region {
    pub fn bounds(&self) -> IRect {
        self.bounds
    }
}

/// Finds all pixels 4-connected to `(x, y)` whose RGBA (premultiplied) is within `tolerance` of
/// the seed pixel on every channel.
pub fn compute_region(pm: &Pixmap, x: i32, y: i32, tolerance: u8) -> Option<Region> {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
        return None;
    }
    let data = pm.data();
    let px = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * w + x) * 4;
        [data[i], data[i + 1], data[i + 2], data[i + 3]]
    };
    let seed = px(x as usize, y as usize);
    let matches = |x: usize, y: usize| -> bool {
        let p = px(x, y);
        (0..4).all(|i| p[i].abs_diff(seed[i]) <= tolerance)
    };

    let mut mask = vec![0u8; w * h];
    let mut stack = vec![(x as usize, y as usize)];
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);

    while let Some((sx, sy)) = stack.pop() {
        if mask[sy * w + sx] != 0 || !matches(sx, sy) {
            continue;
        }
        let mut l = sx;
        while l > 0 && mask[sy * w + l - 1] == 0 && matches(l - 1, sy) {
            l -= 1;
        }
        let mut r = sx;
        while r + 1 < w && mask[sy * w + r + 1] == 0 && matches(r + 1, sy) {
            r += 1;
        }
        for xx in l..=r {
            mask[sy * w + xx] = IN_REGION;
        }
        x0 = x0.min(l);
        x1 = x1.max(r);
        y0 = y0.min(sy);
        y1 = y1.max(sy);

        for ny in [sy.wrapping_sub(1), sy + 1] {
            if ny >= h {
                continue;
            }
            let mut in_run = false;
            for xx in l..=r {
                let m = mask[ny * w + xx] == 0 && matches(xx, ny);
                if m && !in_run {
                    stack.push((xx, ny));
                }
                in_run = m;
            }
        }
    }
    if x0 == usize::MAX {
        return None;
    }
    Some(Region {
        mask,
        bounds: IRect::new(x0 as i32, y0 as i32, x1 as i32 + 1, y1 as i32 + 1),
        seed,
    })
}

/// Expands the region outward onto the anti-aliased fringe of neighbouring ink.
///
/// Pass one takes every non-transparent neighbour of the region. Pass two only follows pixels
/// whose alpha is at least that of the pixel they were reached from, i.e. it climbs the AA ramp
/// toward the opaque core of the line but never descends onto the far side of a thin stroke, so
/// the fill cannot bleed into the next room.
pub fn grow_under_edges(pm: &Pixmap, region: &mut Region) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data();
    let alpha = |x: i32, y: i32| data[((y * w + x) * 4 + 3) as usize];
    let b = region.bounds;

    let mut ring1: Vec<(i32, i32)> = Vec::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            if region.mask[(y * w + x) as usize] != IN_REGION {
                continue;
            }
            for (dx, dy) in NEIGHBOURS {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let i = (ny * w + nx) as usize;
                if region.mask[i] == 0 && alpha(nx, ny) > 0 {
                    region.mask[i] = GROWN;
                    ring1.push((nx, ny));
                }
            }
        }
    }

    let mut ring2: Vec<(i32, i32)> = Vec::new();
    for &(x, y) in &ring1 {
        let a = alpha(x, y);
        for (dx, dy) in NEIGHBOURS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let i = (ny * w + nx) as usize;
            if region.mask[i] == 0 && alpha(nx, ny) > 0 && alpha(nx, ny) >= a {
                region.mask[i] = GROWN;
                ring2.push((nx, ny));
            }
        }
    }
    for &(x, y) in ring1.iter().chain(&ring2) {
        region.bounds = region.bounds.union(&IRect::new(x, y, x + 1, y + 1));
    }
}

const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Paints the region. Pixels reached by flood are replaced only when the seed was real colour
/// (recolouring); against a transparent seed everything, including the grown ring, goes
/// destination-over so existing ink is never covered.
pub fn paint_region(pm: &mut Pixmap, region: &Region, color: [u8; 3], tolerance: u8) {
    let w = pm.width() as usize;
    let replace = region.seed[3] > tolerance;
    let data = pm.data_mut();
    let b = region.bounds;
    for y in b.y0 as usize..b.y1 as usize {
        for x in b.x0 as usize..b.x1 as usize {
            let m = region.mask[y * w + x];
            if m == 0 {
                continue;
            }
            let i = (y * w + x) * 4;
            if m == IN_REGION && replace {
                data[i..i + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
                continue;
            }
            let a = data[i + 3];
            if a == 255 {
                continue;
            }
            let inv = (255 - a) as u32;
            for c in 0..3 {
                data[i + c] = (data[i + c] as u32 + (color[c] as u32 * inv + 127) / 255).min(255) as u8;
            }
            data[i + 3] = 255;
        }
    }
}

/// Fills at `(x, y)` on `frame`, returning an undo entry for the changed rectangle.
pub fn flood_fill(frame: &mut Frame, x: i32, y: i32, color: [u8; 3], tolerance: u8) -> Option<PixelEdit> {
    let mut region = compute_region(frame.pixmap_mut(), x, y, tolerance)?;
    grow_under_edges(frame.pixmap()?, &mut region);
    let rect = region.bounds.clamp_to(frame.width(), frame.height());
    let before = extract_rect(frame.pixmap()?, rect);
    paint_region(frame.pixmap_mut(), &region, color, tolerance);
    let after = extract_rect(frame.pixmap()?, rect);
    if before == after {
        return None;
    }
    frame.touch(Some(rect));
    Some(PixelEdit {
        frame: frame.id,
        rect,
        before,
        after,
        label: "Fill",
    })
}

pub struct FillTool;

impl Tool for FillTool {
    fn id(&self) -> ToolId {
        ToolId::Fill
    }

    fn begin(&mut self, frame: &mut Frame, s: &ToolSettings, at: Sample) -> ToolOutput {
        match flood_fill(frame, at.x.floor() as i32, at.y.floor() as i32, s.color, s.tolerance) {
            Some(edit) => ToolOutput::Commit(Box::new(edit)),
            None => ToolOutput::None,
        }
    }

    fn drag(&mut self, _f: &mut Frame, _s: &ToolSettings, _at: Sample) {}

    fn end(&mut self, _f: &mut Frame, _s: &ToolSettings, _at: Sample) -> ToolOutput {
        ToolOutput::None
    }

    fn cancel(&mut self, _f: &mut Frame) {}

    fn is_active(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FrameId;
    use tiny_skia::{Color, Paint, PathBuilder, Stroke, Transform};

    const C: f32 = 50.0;
    const R: f32 = 30.0;

    fn ring_frame(width: f32) -> Frame {
        let mut f = Frame::blank(FrameId(1), 100, 100);
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(20, 20, 30, 255));
        paint.anti_alias = true;
        let path = PathBuilder::from_circle(C + 0.3, C - 0.2, R).unwrap();
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        f.pixmap_mut()
            .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        f
    }

    fn dist(x: u32, y: u32) -> f32 {
        let (dx, dy) = (x as f32 + 0.5 - (C + 0.3), y as f32 + 0.5 - (C - 0.2));
        (dx * dx + dy * dy).sqrt()
    }

    fn alpha(f: &Frame, x: u32, y: u32) -> u8 {
        f.pixmap().unwrap().pixel(x, y).unwrap().alpha()
    }

    #[test]
    fn antialiased_ring_has_partial_pixels_to_begin_with() {
        let f = ring_frame(4.0);
        let partial = (0..100u32)
            .flat_map(|y| (0..100u32).map(move |x| (x, y)))
            .filter(|&(x, y)| (1..255).contains(&(alpha(&f, x, y) as u32)))
            .count();
        assert!(partial > 40, "test needs an anti-aliased border, got {partial}");
    }

    #[test]
    fn fill_leaves_no_halo_against_antialiased_border() {
        let mut f = ring_frame(4.0);
        let edit = flood_fill(&mut f, 50, 50, [230, 80, 40], 32).expect("fill happens");
        assert!(edit.rect.width() < 100, "dirty rect is bounded: {:?}", edit.rect);

        let outer = R + 2.0;
        for y in 0..100 {
            for x in 0..100 {
                let d = dist(x, y);
                if d <= outer - 0.9 {
                    assert_eq!(alpha(&f, x, y), 255, "see-through pixel (halo) at {x},{y} d={d}");
                }
                if d > outer + 1.2 {
                    assert_eq!(alpha(&f, x, y), 0, "fill leaked outside at {x},{y} d={d}");
                }
            }
        }
        // The line itself stays on top: its core is still the dark ink colour.
        let core = f.pixmap().unwrap().pixel((C + R) as u32, C as u32).unwrap();
        assert!(core.red() < 60, "ink not covered: {:?}", core);
        // Interior is the fill colour.
        let mid = f.pixmap().unwrap().pixel(50, 50).unwrap();
        assert_eq!((mid.red(), mid.green(), mid.blue(), mid.alpha()), (230, 80, 40, 255));
    }

    #[test]
    fn without_tucking_under_there_would_be_a_halo() {
        // Guards the test above: plain flood (no growth) does leave see-through edge pixels.
        let mut f = ring_frame(4.0);
        let region = compute_region(f.pixmap().unwrap(), 50, 50, 32).unwrap();
        paint_region(f.pixmap_mut(), &region, [230, 80, 40], 32);
        let halo = (0..100u32)
            .flat_map(|y| (0..100u32).map(move |x| (x, y)))
            .filter(|&(x, y)| dist(x, y) < R + 1.1 && alpha(&f, x, y) < 255)
            .count();
        assert!(halo > 10, "expected a halo without tuck-under, got {halo}");
    }

    #[test]
    fn thin_lines_do_not_bleed_outside() {
        let mut f = ring_frame(1.2);
        flood_fill(&mut f, 50, 50, [10, 200, 90], 32).unwrap();
        for y in 0..100 {
            for x in 0..100 {
                if dist(x, y) > R + 1.8 {
                    assert_eq!(alpha(&f, x, y), 0, "bled outside at {x},{y}");
                }
            }
        }
        assert_eq!(alpha(&f, 50, 50), 255);
    }

    #[test]
    fn filling_outside_the_shape_does_not_enter_it() {
        let mut f = ring_frame(4.0);
        flood_fill(&mut f, 2, 2, [0, 0, 255], 32).unwrap();
        assert_eq!(alpha(&f, 2, 2), 255);
        assert_eq!(alpha(&f, 50, 50), 0, "interior untouched");
    }

    #[test]
    fn refilling_recolours_the_region() {
        let mut f = ring_frame(4.0);
        flood_fill(&mut f, 50, 50, [255, 0, 0], 32).unwrap();
        flood_fill(&mut f, 50, 50, [0, 0, 255], 32).unwrap();
        let p = f.pixmap().unwrap().pixel(50, 50).unwrap();
        assert_eq!((p.red(), p.blue()), (0, 255));
        // Same colour again is a no-op and produces no history entry.
        assert!(flood_fill(&mut f, 50, 50, [0, 0, 255], 32).is_none());
    }

    #[test]
    fn tolerance_widens_the_region() {
        let mut f = Frame::blank(FrameId(1), 20, 4);
        // Left half opaque grey 100, right half opaque grey 120.
        for y in 0..4usize {
            for x in 0..20usize {
                let g = if x < 10 { 100 } else { 120 };
                let i = (y * 20 + x) * 4;
                f.pixmap_mut().data_mut()[i..i + 4].copy_from_slice(&[g, g, g, 255]);
            }
        }
        let tight = compute_region(f.pixmap().unwrap(), 0, 0, 5).unwrap();
        assert_eq!(tight.bounds().x1, 10);
        let loose = compute_region(f.pixmap().unwrap(), 0, 0, 25).unwrap();
        assert_eq!(loose.bounds().x1, 20);
    }

    #[test]
    fn fill_does_not_leak_through_diagonal_gaps() {
        // A diagonal wall of single pixels: 4-connectivity must not squeeze between them.
        let mut f = Frame::blank(FrameId(1), 10, 10);
        for i in 0..10usize {
            let idx = (i * 10 + i) * 4;
            f.pixmap_mut().data_mut()[idx..idx + 4].copy_from_slice(&[0, 0, 0, 255]);
        }
        let r = compute_region(f.pixmap().unwrap(), 9, 0, 0).unwrap();
        assert_eq!(r.mask[0 * 10 + 1], IN_REGION);
        assert_eq!(r.mask[1 * 10 + 0], 0, "other side of the diagonal wall");
    }
}
