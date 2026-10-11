//! Compositing and GPU caches: per-frame texture sync (partial uploads), the onion-skin
//! composite (rebuilt only when its inputs change) and timeline thumbnails.

use eframe::egui::{Color32, ColorImage, Context, TextureFilter, TextureHandle, TextureOptions};
use tiny_skia::Pixmap;

use crate::model::pixels::extract_rect;
use crate::model::{Dirty, Frame, FrameId, Project};

pub const CANVAS_TEXTURE: TextureOptions = TextureOptions {
    magnification: TextureFilter::Nearest,
    minification: TextureFilter::Linear,
    wrap_mode: eframe::egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

pub const THUMB_HEIGHT: u32 = 72;

fn color_image(w: u32, h: u32, premultiplied_rgba: &[u8]) -> ColorImage {
    ColorImage::from_rgba_premultiplied([w as usize, h as usize], premultiplied_rgba)
}

/// Uploads whatever changed since the last call. Does nothing when the frame is clean, uploads
/// only the dirty rectangle during strokes, and skips blank frames entirely.
/// Returns the texture to draw, if the frame has any content.
pub fn sync_texture(ctx: &Context, frame: &mut Frame) -> Option<TextureHandle> {
    let Some(pm) = frame.pixmap_arc() else {
        frame.clear_dirty();
        frame.gpu.texture = None;
        return None;
    };
    let pm = &*pm;
    let (w, h) = (pm.width(), pm.height());
    let dirty = frame.dirty();
    let id = frame.id;
    let result = match (&mut frame.gpu.texture, dirty) {
        (Some(tex), Dirty::Clean) => Some(tex.clone()),
        (Some(tex), Dirty::Rect(r)) => {
            let r = r.clamp_to(w, h);
            if !r.is_empty() {
                let bytes = extract_rect(pm, r);
                let img = color_image(r.width(), r.height(), &bytes);
                tex.set_partial([r.x0 as usize, r.y0 as usize], img, CANVAS_TEXTURE);
            }
            Some(tex.clone())
        }
        (slot, _) => {
            let img = color_image(w, h, pm.data());
            let handle = match slot.as_mut() {
                Some(tex) => {
                    tex.set(img, CANVAS_TEXTURE);
                    tex.clone()
                }
                None => ctx.load_texture(format!("frame-{}", id.0), img, CANVAS_TEXTURE),
            };
            *slot = Some(handle.clone());
            Some(handle)
        }
    };
    frame.clear_dirty();
    result
}

/// Frees GPU textures for frames away from the playhead so 100-frame projects don't hold
/// 100 full-size textures while editing.
pub fn evict_far_textures(project: &mut Project, current: usize, keep_radius: usize) {
    for (i, f) in project.frames_mut().iter_mut().enumerate() {
        if i.abs_diff(current) > keep_radius && f.gpu.texture.is_some() {
            f.invalidate_gpu();
        }
    }
}

pub fn free_all_but(project: &mut Project, current: usize) {
    evict_far_textures(project, current, 0);
}

// ---------------------------------------------------------------------------------------------
// Onion skin
// ---------------------------------------------------------------------------------------------

pub const ONION_PAST: [u8; 3] = [0xE5, 0x48, 0x4D];
pub const ONION_FUTURE: [u8; 3] = [0x2E, 0xA8, 0x5B];
/// Opacity at distance 1, 2, 3 frames from the current one.
const ONION_FADE: [f32; 3] = [0.55, 0.32, 0.17];

pub const MIN_ONION_DEPTH: u8 = 1;
pub const MAX_ONION_DEPTH: u8 = 3;

/// One neighbour layer for [`compose_onion`]: signed distance (negative = past) and pixels.
pub struct OnionLayer<'a> {
    pub distance: i32,
    pub pixmap: Option<&'a Pixmap>,
}

/// Flattens neighbour frames into one tinted, distance-faded layer (past red, future green).
pub fn compose_onion(w: u32, h: u32, layers: &[OnionLayer]) -> Pixmap {
    let mut out = Pixmap::new(w, h).expect("non-zero canvas");
    // Farthest first so nearer frames sit on top.
    let mut order: Vec<&OnionLayer> = layers.iter().collect();
    order.sort_by_key(|l| std::cmp::Reverse(l.distance.abs()));
    for layer in order {
        let Some(src) = layer.pixmap else { continue };
        let d = layer.distance.unsigned_abs().clamp(1, 3) as usize;
        let fade = (ONION_FADE[d - 1] * 255.0).round() as u32;
        let tint = if layer.distance < 0 {
            ONION_PAST
        } else {
            ONION_FUTURE
        };
        let dst = out.data_mut();
        for (s, o) in src.data().chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            let a = s[3] as u32;
            if a == 0 {
                continue;
            }
            let sa = (a * fade + 127) / 255;
            let inv = 255 - sa;
            for c in 0..3 {
                let sc = (tint[c] as u32 * sa + 127) / 255;
                o[c] = (sc + (o[c] as u32 * inv + 127) / 255).min(255) as u8;
            }
            o[3] = (sa + (o[3] as u32 * inv + 127) / 255).min(255) as u8;
        }
    }
    out
}

/// Everything the onion composite depends on. When this is unchanged the cached texture is
/// reused; note the *current* frame's pixels are deliberately absent, so drawing never rebuilds it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OnionKey {
    pub current: FrameId,
    pub depth: u8,
    pub size: (u32, u32),
    pub neighbours: Vec<(i32, FrameId, u64)>,
}

pub fn onion_key(project: &Project, current: usize, depth: u8) -> Option<OnionKey> {
    let cur = project.frame(current)?;
    let mut neighbours = Vec::new();
    for d in 1..=depth as i32 {
        for sign in [-1i32, 1] {
            let idx = current as i32 + sign * d;
            if idx >= 0 {
                if let Some(f) = project.frame(idx as usize) {
                    neighbours.push((sign * d, f.id, f.rev()));
                }
            }
        }
    }
    Some(OnionKey {
        current: cur.id,
        depth,
        size: (project.width(), project.height()),
        neighbours,
    })
}

#[derive(Default)]
pub struct OnionCache {
    key: Option<OnionKey>,
    texture: Option<TextureHandle>,
    pub rebuilds: u64,
}

impl OnionCache {
    /// Returns the onion texture for the current state, rebuilding only if the key changed.
    pub fn get(
        &mut self,
        ctx: &Context,
        project: &Project,
        current: usize,
        depth: u8,
    ) -> Option<TextureHandle> {
        let key = onion_key(project, current, depth)?;
        if self.key.as_ref() == Some(&key) {
            return self.texture.clone();
        }
        let layers: Vec<OnionLayer> = key
            .neighbours
            .iter()
            .filter_map(|(d, id, _)| {
                let f = project.frame(project.index_of(*id)?)?;
                Some(OnionLayer {
                    distance: *d,
                    pixmap: f.pixmap(),
                })
            })
            .collect();
        let any = layers.iter().any(|l| l.pixmap.is_some());
        self.texture = if any {
            let pm = compose_onion(project.width(), project.height(), &layers);
            let img = color_image(pm.width(), pm.height(), pm.data());
            match self.texture.take() {
                Some(mut t) if t.size() == [pm.width() as usize, pm.height() as usize] => {
                    t.set(img, CANVAS_TEXTURE);
                    Some(t)
                }
                _ => Some(ctx.load_texture("onion", img, CANVAS_TEXTURE)),
            }
        } else {
            None
        };
        self.key = Some(key);
        self.rebuilds += 1;
        self.texture.clone()
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

// ---------------------------------------------------------------------------------------------
// Thumbnails
// ---------------------------------------------------------------------------------------------

pub fn thumb_size(w: u32, h: u32) -> (u32, u32) {
    let th = THUMB_HEIGHT;
    let tw = ((w as f32 / h as f32) * th as f32).round().clamp(16.0, 160.0) as u32;
    (tw, th)
}

/// Box-filter downscale composited over the paper colour.
pub fn make_thumbnail(pm: Option<&Pixmap>, w: u32, h: u32, paper: [u8; 3]) -> ColorImage {
    let (tw, th) = thumb_size(w, h);
    let mut rgba = vec![0u8; (tw * th * 4) as usize];
    for ty in 0..th {
        let y0 = (ty as u64 * h as u64 / th as u64) as usize;
        let y1 = (((ty + 1) as u64 * h as u64 / th as u64) as usize)
            .max(y0 + 1)
            .min(h as usize);
        for tx in 0..tw {
            let x0 = (tx as u64 * w as u64 / tw as u64) as usize;
            let x1 = (((tx + 1) as u64 * w as u64 / tw as u64) as usize)
                .max(x0 + 1)
                .min(w as usize);
            let mut sum = [0u32; 4];
            if let Some(pm) = pm {
                let data = pm.data();
                for y in y0..y1 {
                    let row = &data[(y * w as usize + x0) * 4..(y * w as usize + x1) * 4];
                    for px in row.chunks_exact(4) {
                        sum[0] += px[0] as u32;
                        sum[1] += px[1] as u32;
                        sum[2] += px[2] as u32;
                        sum[3] += px[3] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as u32;
            let a = sum[3] / n;
            let inv = 255 - a;
            let o = ((ty * tw + tx) * 4) as usize;
            for c in 0..3 {
                rgba[o + c] = ((sum[c] / n) + (paper[c] as u32 * inv + 127) / 255).min(255) as u8;
            }
            rgba[o + 3] = 255;
        }
    }
    ColorImage::from_rgba_premultiplied([tw as usize, th as usize], &rgba)
}

/// Refreshes up to `budget` stale thumbnails; returns true if more remain (caller repaints).
pub fn update_thumbnails(ctx: &Context, project: &mut Project, skip: Option<FrameId>, budget: usize) -> bool {
    let (w, h, paper) = (project.width(), project.height(), project.paper);
    let mut done = 0;
    let mut pending = false;
    for f in project.frames_mut() {
        let stale = f.gpu.thumb.is_none() || f.gpu.thumb_rev != f.rev();
        if !stale {
            continue;
        }
        if Some(f.id) == skip && f.gpu.thumb.is_some() {
            continue;
        }
        if done >= budget {
            pending = true;
            continue;
        }
        let img = make_thumbnail(f.pixmap(), w, h, paper);
        match &mut f.gpu.thumb {
            Some(t) => t.set(img, TextureOptions::LINEAR),
            None => {
                f.gpu.thumb = Some(ctx.load_texture(format!("thumb-{}", f.id.0), img, TextureOptions::LINEAR))
            }
        }
        f.gpu.thumb_rev = f.rev();
        done += 1;
    }
    pending
}

pub fn color32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FrameId;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
        let mut pm = Pixmap::new(w, h).unwrap();
        for p in pm.data_mut().chunks_exact_mut(4) {
            p.copy_from_slice(&rgba);
        }
        pm
    }

    #[test]
    fn onion_tints_past_red_future_green_and_fades_with_distance() {
        let black = solid(4, 4, [0, 0, 0, 255]);
        let layers = [
            OnionLayer {
                distance: -1,
                pixmap: Some(&black),
            },
            OnionLayer {
                distance: -3,
                pixmap: Some(&black),
            },
        ];
        let out = compose_onion(4, 4, &layers);
        let near = out.pixel(0, 0).unwrap();
        assert!(
            near.red() > near.green() && near.red() > near.blue(),
            "past is red"
        );

        let only_far = compose_onion(
            4,
            4,
            &[OnionLayer {
                distance: -3,
                pixmap: Some(&black),
            }],
        );
        let only_near = compose_onion(
            4,
            4,
            &[OnionLayer {
                distance: -1,
                pixmap: Some(&black),
            }],
        );
        assert!(only_far.pixel(0, 0).unwrap().alpha() < only_near.pixel(0, 0).unwrap().alpha());

        let fut = compose_onion(
            4,
            4,
            &[OnionLayer {
                distance: 2,
                pixmap: Some(&black),
            }],
        );
        let p = fut.pixel(0, 0).unwrap();
        assert!(p.green() > p.red(), "future is green");
    }

    #[test]
    fn onion_ignores_transparent_pixels() {
        let clear = Pixmap::new(4, 4).unwrap();
        let out = compose_onion(
            4,
            4,
            &[OnionLayer {
                distance: 1,
                pixmap: Some(&clear),
            }],
        );
        assert!(out.data().iter().all(|&b| b == 0));
    }

    #[test]
    fn onion_key_ignores_current_frame_edits_but_tracks_neighbours() {
        let mut p = Project::new("t", 8, 8);
        for _ in 0..4 {
            let f = p.new_blank_frame();
            p.insert_frame(p.len(), f);
        }
        let k0 = onion_key(&p, 2, 2).unwrap();
        p.frame_mut(2).unwrap().touch(None); // drawing on the current frame
        assert_eq!(
            onion_key(&p, 2, 2).unwrap(),
            k0,
            "current frame's pixels are not an input"
        );

        p.frame_mut(1).unwrap().touch(None); // neighbour changed
        assert_ne!(onion_key(&p, 2, 2).unwrap(), k0);

        assert_ne!(
            onion_key(&p, 2, 1).unwrap(),
            onion_key(&p, 2, 2).unwrap(),
            "depth is an input"
        );
        assert_ne!(
            onion_key(&p, 3, 2).unwrap(),
            onion_key(&p, 2, 2).unwrap(),
            "current index is an input"
        );
    }

    #[test]
    fn onion_key_far_frames_beyond_depth_do_not_matter() {
        let mut p = Project::new("t", 8, 8);
        for _ in 0..6 {
            let f = p.new_blank_frame();
            p.insert_frame(p.len(), f);
        }
        let k = onion_key(&p, 3, 1).unwrap();
        p.frame_mut(0).unwrap().touch(None);
        p.frame_mut(6).unwrap().touch(None);
        assert_eq!(onion_key(&p, 3, 1).unwrap(), k);
    }

    #[test]
    fn thumbnail_keeps_aspect_and_shows_paper_for_blank() {
        let img = make_thumbnail(None, 1280, 720, [250, 247, 238]);
        assert_eq!(img.size, [128, 72]);
        assert_eq!(img.pixels[0], Color32::from_rgb(250, 247, 238));
        let img = make_thumbnail(None, 720, 1280, [0, 0, 0]);
        assert_eq!(img.size[1], 72);
        assert!((40..=41).contains(&img.size[0]));
    }

    #[test]
    fn thumbnail_averages_content() {
        let mut pm = Pixmap::new(100, 100).unwrap();
        for y in 0..100usize {
            for x in 0..50usize {
                let i = (y * 100 + x) * 4;
                pm.data_mut()[i..i + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        let img = make_thumbnail(Some(&pm), 100, 100, [255, 255, 255]);
        let w = img.size[0];
        assert_eq!(img.pixels[5], Color32::from_rgb(0, 0, 0));
        assert_eq!(img.pixels[w - 5], Color32::from_rgb(255, 255, 255));
    }

    #[test]
    fn frame_ids_in_key_follow_neighbours_not_indices() {
        let mut p = Project::new("t", 8, 8);
        let f = p.new_blank_frame();
        p.insert_frame(1, f);
        let k = onion_key(&p, 0, 1).unwrap();
        assert_eq!(k.neighbours.len(), 1);
        assert_eq!(k.neighbours[0].0, 1);
        assert_ne!(k.neighbours[0].1, FrameId(0));
    }
}
