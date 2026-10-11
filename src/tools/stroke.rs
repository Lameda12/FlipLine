//! Brush/eraser engine: input smoothing, pressure-to-width mapping and capsule rasterisation.

use std::sync::Arc;

use tiny_skia::{BlendMode, Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

use super::{Sample, ToolSettings};
use crate::history::PixelEdit;
use crate::model::pixels::{extract_rect, IRect};
use crate::model::{Frame, FrameId};

/// Narrowest the stroke gets at zero pressure, as a fraction of the full width.
const MIN_PRESSURE_WIDTH: f32 = 0.12;
/// How strongly raw input is pulled toward the new sample (1.0 = no smoothing).
const SMOOTHING: f32 = 0.55;

pub fn radius_for(size: f32, pressure: f32) -> f32 {
    let p = pressure.clamp(0.0, 1.0);
    let f = MIN_PRESSURE_WIDTH + (1.0 - MIN_PRESSURE_WIDTH) * p;
    (size * 0.5 * f).max(0.5)
}

#[derive(Clone, Copy)]
struct P {
    x: f32,
    y: f32,
    pr: f32,
}

impl P {
    fn from(s: Sample) -> Self {
        P {
            x: s.x,
            y: s.y,
            pr: s.pressure,
        }
    }

    fn lerp(a: P, b: P, t: f32) -> P {
        P {
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
            pr: a.pr + (b.pr - a.pr) * t,
        }
    }

    fn dist(a: P, b: P) -> f32 {
        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
    }
}

/// Turns jittery input into a smooth curve: exponential smoothing, then a quadratic spline
/// through segment midpoints (C1 continuous), sampled finely enough for capsule stamping.
pub struct Smoother {
    smoothed: Option<P>,
    ctrl: P,
    mid: P,
    spacing: f32,
}

impl Smoother {
    pub fn new(spacing: f32) -> Self {
        let z = P {
            x: 0.0,
            y: 0.0,
            pr: 1.0,
        };
        Self {
            smoothed: None,
            ctrl: z,
            mid: z,
            spacing: spacing.max(0.5),
        }
    }

    pub fn begin(&mut self, s: Sample) -> Vec<Sample> {
        let p = P::from(s);
        self.smoothed = Some(p);
        self.ctrl = p;
        self.mid = p;
        vec![s]
    }

    pub fn push(&mut self, s: Sample) -> Vec<Sample> {
        let Some(prev) = self.smoothed else {
            return self.begin(s);
        };
        let next = P::lerp(prev, P::from(s), SMOOTHING);
        if P::dist(prev, next) < 0.05 {
            return Vec::new();
        }
        let mid = P::lerp(self.ctrl, next, 0.5);
        let out = self.curve(self.mid, self.ctrl, mid);
        self.mid = mid;
        self.ctrl = next;
        self.smoothed = Some(next);
        out
    }

    /// Closes the stroke exactly at the last raw position so it never stops short of the cursor.
    pub fn finish(&mut self, last: Sample) -> Vec<Sample> {
        if self.smoothed.is_none() {
            return Vec::new();
        }
        let end = P::from(last);
        let out = self.curve(self.mid, self.ctrl, end);
        self.mid = end;
        self.smoothed = None;
        out
    }

    fn curve(&self, p0: P, c: P, p1: P) -> Vec<Sample> {
        let len = P::dist(p0, c) + P::dist(c, p1);
        let n = ((len / self.spacing).ceil() as usize).clamp(1, 64);
        (1..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let a = P::lerp(p0, c, t);
                let b = P::lerp(c, p1, t);
                let q = P::lerp(a, b, t);
                Sample::new(q.x, q.y, q.pr)
            })
            .collect()
    }
}

fn arc_steps(radius: f32, sweep: f32) -> usize {
    ((radius * sweep.abs() / 1.6).ceil() as usize).clamp(4, 48)
}

/// Convex hull of two circles (a tapered capsule) as one polygon, so each stroke segment is a
/// single anti-aliased fill instead of three overlapping ones.
fn capsule_path(a: (f32, f32, f32), b: (f32, f32, f32)) -> Option<tiny_skia::Path> {
    let (ax, ay, ra) = a;
    let (bx, by, rb) = b;
    let (dx, dy) = (bx - ax, by - ay);
    let dist = (dx * dx + dy * dy).sqrt();
    let mut pb = PathBuilder::new();
    if dist <= (ra - rb).abs() + 1e-3 {
        let (cx, cy, r) = if ra >= rb { (ax, ay, ra) } else { (bx, by, rb) };
        return PathBuilder::from_circle(cx, cy, r);
    }
    let theta = dy.atan2(dx);
    let phi = ((ra - rb) / dist).clamp(-1.0, 1.0).acos();
    let tau = std::f32::consts::TAU;

    // Back arc of circle a, from theta+phi round the rear to theta-phi (+2π).
    let (a0, a1) = (theta + phi, theta + tau - phi);
    let n = arc_steps(ra, a1 - a0);
    for i in 0..=n {
        let ang = a0 + (a1 - a0) * i as f32 / n as f32;
        let (x, y) = (ax + ra * ang.cos(), ay + ra * ang.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    // Front arc of circle b, from theta-phi round the front to theta+phi.
    let (b0, b1) = (theta - phi, theta + phi);
    let n = arc_steps(rb, b1 - b0);
    for i in 0..=n {
        let ang = b0 + (b1 - b0) * i as f32 / n as f32;
        pb.line_to(bx + rb * ang.cos(), by + rb * ang.sin());
    }
    pb.close();
    pb.finish()
}

/// Draws a dot or a tapered segment. Returns the pixel bounds touched.
pub fn stamp(
    pm: &mut Pixmap,
    paint: &Paint,
    a: (f32, f32, f32),
    b: Option<(f32, f32, f32)>,
) -> IRect {
    let path = match b {
        Some(b) => capsule_path(a, b),
        None => PathBuilder::from_circle(a.0, a.1, a.2),
    };
    let r = b.map_or(a.2, |b| a.2.max(b.2));
    let (minx, miny, maxx, maxy) = match b {
        Some(b) => (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)),
        None => (a.0, a.1, a.0, a.1),
    };
    if let Some(path) = path {
        pm.fill_path(&path, paint, FillRule::Winding, Transform::identity(), None);
    }
    IRect::new(
        (minx - r).floor() as i32 - 2,
        (miny - r).floor() as i32 - 2,
        (maxx + r).ceil() as i32 + 2,
        (maxy + r).ceil() as i32 + 2,
    )
}

/// State of one brush/eraser gesture.
pub struct StrokeEngine {
    erase: bool,
    active: Option<Active>,
}

struct Active {
    frame: FrameId,
    /// Pixels before the stroke began (`None` = frame was blank).
    before: Option<Arc<Pixmap>>,
    smoother: Smoother,
    bounds: IRect,
    last: (f32, f32, f32),
    last_raw: Sample,
    size: f32,
    paint: Paint<'static>,
}

impl StrokeEngine {
    pub fn new(erase: bool) -> Self {
        Self {
            erase,
            active: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    fn make_paint(&self, s: &ToolSettings) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        if self.erase {
            paint.blend_mode = BlendMode::Clear;
            paint.set_color(Color::BLACK);
        } else {
            paint.set_color_rgba8(s.color[0], s.color[1], s.color[2], 255);
        }
        paint
    }

    pub fn begin(&mut self, frame: &mut Frame, s: &ToolSettings, at: Sample) {
        let before = frame.pixmap_arc();
        let spacing = (s.size * 0.25).clamp(1.0, 5.0);
        let mut smoother = Smoother::new(spacing);
        let first = smoother.begin(at)[0];
        let paint = self.make_paint(s);
        let r = radius_for(s.size, first.pressure);
        let last = (first.x, first.y, r);
        let bounds = stamp(frame.pixmap_mut(), &paint, last, None);
        frame.touch(Some(bounds));
        self.active = Some(Active {
            frame: frame.id,
            before,
            smoother,
            bounds,
            last,
            last_raw: at,
            size: s.size,
            paint,
        });
    }

    fn draw_points(a: &mut Active, frame: &mut Frame, points: Vec<Sample>) {
        if points.is_empty() {
            return;
        }
        let mut dirty: Option<IRect> = None;
        for p in points {
            let cur = (p.x, p.y, radius_for(a.size, p.pressure));
            let r = stamp(frame.pixmap_mut(), &a.paint, a.last, Some(cur));
            a.last = cur;
            dirty = Some(dirty.map_or(r, |d| d.union(&r)));
        }
        if let Some(d) = dirty {
            a.bounds = a.bounds.union(&d);
            frame.touch(Some(d));
        }
    }

    pub fn drag(&mut self, frame: &mut Frame, at: Sample) {
        if let Some(a) = self.active.as_mut() {
            if a.frame != frame.id {
                return;
            }
            a.last_raw = at;
            let pts = a.smoother.push(at);
            Self::draw_points(a, frame, pts);
        }
    }

    /// Finishes the stroke and returns the undo entry (only the dirty rect's before/after).
    pub fn end(&mut self, frame: &mut Frame, at: Sample) -> Option<PixelEdit> {
        let mut a = self.active.take()?;
        if a.frame != frame.id {
            return None;
        }
        let pts = a.smoother.finish(at);
        Self::draw_points(&mut a, frame, pts);

        let rect = a.bounds.clamp_to(frame.width(), frame.height());
        if rect.is_empty() {
            return None;
        }
        let before = match &a.before {
            Some(pm) => extract_rect(pm, rect),
            None => vec![0; rect.area_bytes()],
        };
        let after = extract_rect(frame.pixmap()?, rect);
        if before == after {
            return None;
        }
        Some(PixelEdit {
            frame: a.frame,
            rect,
            before,
            after,
            label: if self.erase { "Erase" } else { "Brush stroke" },
        })
    }

    pub fn cancel(&mut self, frame: &mut Frame) {
        let Some(a) = self.active.take() else { return };
        if a.frame != frame.id {
            return;
        }
        let rect = a.bounds.clamp_to(frame.width(), frame.height());
        let before = match &a.before {
            Some(pm) => extract_rect(pm, rect),
            None => vec![0; rect.area_bytes()],
        };
        crate::model::pixels::write_rect(frame.pixmap_mut(), rect, &before);
        frame.touch(Some(rect));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_stroke(erase: bool, frame: &mut Frame, pts: &[(f32, f32, f32)]) -> Option<PixelEdit> {
        let s = ToolSettings {
            size: 10.0,
            ..ToolSettings::default()
        };
        let mut e = StrokeEngine::new(erase);
        let (x, y, p) = pts[0];
        e.begin(frame, &s, Sample::new(x, y, p));
        for &(x, y, p) in &pts[1..] {
            e.drag(frame, Sample::new(x, y, p));
        }
        let (x, y, p) = *pts.last().unwrap();
        e.end(frame, Sample::new(x, y, p))
    }

    #[test]
    fn smoother_ends_exactly_at_last_input_and_stays_near_path() {
        let mut s = Smoother::new(2.0);
        let mut all = s.begin(Sample::new(0.0, 0.0, 1.0));
        for i in 1..=20 {
            all.extend(s.push(Sample::new(i as f32 * 5.0, (i as f32 * 0.7).sin() * 4.0, 1.0)));
        }
        all.extend(s.finish(Sample::new(100.0, 2.0, 1.0)));
        let last = all.last().unwrap();
        assert!((last.x - 100.0).abs() < 1e-3 && (last.y - 2.0).abs() < 1e-3);
        for w in all.windows(2) {
            let d = ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt();
            assert!(d < 4.5, "samples are densely spaced, got gap {d}");
        }
    }

    #[test]
    fn pressure_maps_to_width_and_defaults_to_full() {
        assert!((radius_for(20.0, 1.0) - 10.0).abs() < 1e-5);
        assert!(radius_for(20.0, 0.1) < radius_for(20.0, 0.9));
        assert!(radius_for(1.0, 0.0) >= 0.5);
    }

    #[test]
    fn stroke_draws_and_produces_a_rect_limited_undo_entry() {
        let mut f = Frame::blank(FrameId(1), 200, 100);
        let edit = run_stroke(false, &mut f, &[(20.0, 50.0, 1.0), (60.0, 50.0, 1.0), (100.0, 50.0, 1.0)]).unwrap();
        let pm = f.pixmap().unwrap();
        assert_eq!(pm.pixel(60, 50).unwrap().alpha(), 255);
        assert_eq!(pm.pixel(60, 10).unwrap().alpha(), 0);
        assert!(edit.rect.width() < 200 && edit.rect.height() < 30, "{:?}", edit.rect);
        assert_eq!(edit.before.len(), edit.rect.area_bytes());
        assert!(edit.before.iter().all(|&b| b == 0));
    }

    #[test]
    fn variable_pressure_gives_a_thicker_end_than_start() {
        let mut f = Frame::blank(FrameId(1), 200, 100);
        run_stroke(false, &mut f, &[(20.0, 50.0, 0.1), (60.0, 50.0, 0.5), (110.0, 50.0, 1.0), (130.0, 50.0, 1.0)]).unwrap();
        let pm = f.pixmap().unwrap();
        let thickness = |x: u32| (0..100).filter(|&y| pm.pixel(x, y).unwrap().alpha() > 128).count();
        assert!(thickness(110) > thickness(30) + 2, "{} vs {}", thickness(110), thickness(30));
    }

    #[test]
    fn eraser_clears_pixels_and_cancel_restores_them() {
        let mut f = Frame::blank(FrameId(1), 100, 100);
        run_stroke(false, &mut f, &[(10.0, 50.0, 1.0), (90.0, 50.0, 1.0)]).unwrap();
        assert_eq!(f.pixmap().unwrap().pixel(50, 50).unwrap().alpha(), 255);
        let snapshot = f.pixmap().unwrap().data().to_vec();

        run_stroke(true, &mut f, &[(50.0, 40.0, 1.0), (50.0, 60.0, 1.0)]).unwrap();
        assert_eq!(f.pixmap().unwrap().pixel(50, 50).unwrap().alpha(), 0);

        // Cancelling mid-gesture puts the pixels back.
        let s = ToolSettings::default();
        let mut e = StrokeEngine::new(true);
        let before_cancel = f.pixmap().unwrap().data().to_vec();
        e.begin(&mut f, &s, Sample::new(20.0, 50.0, 1.0));
        e.drag(&mut f, Sample::new(30.0, 50.0, 1.0));
        e.cancel(&mut f);
        assert_eq!(f.pixmap().unwrap().data(), &before_cancel[..]);
        assert_ne!(before_cancel, snapshot);
    }

    #[test]
    fn single_click_makes_a_dot() {
        let mut f = Frame::blank(FrameId(1), 50, 50);
        let e = run_stroke(false, &mut f, &[(25.0, 25.0, 1.0)]).unwrap();
        assert_eq!(f.pixmap().unwrap().pixel(25, 25).unwrap().alpha(), 255);
        assert!(e.rect.width() <= 20);
    }
}
