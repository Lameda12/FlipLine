//! The first-launch sample: a 12-frame bouncing ball with squash on contact, stretch near the
//! ground and a shadow that scales with height. Generated in code, so it ships no assets.

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::model::Project;

pub const W: u32 = 960;
pub const H: u32 = 540;
pub const FRAMES: usize = 12;

const GROUND_Y: f32 = 450.0;
const RADIUS: f32 = 56.0;
const APEX: f32 = 250.0;
const CX: f32 = W as f32 / 2.0;

/// Pose of the ball in frame `i` of the loop. Frame 0 is the contact frame.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    /// Height of the bottom of the ball above the ground.
    pub height: f32,
    pub sx: f32,
    pub sy: f32,
}

pub fn pose(i: usize) -> Pose {
    let u = i as f32 / FRAMES as f32;
    let height = APEX * 4.0 * u * (1.0 - u);
    let speed = (1.0 - 2.0 * u).abs();
    let contact = (1.0 - height / 14.0).clamp(0.0, 1.0);
    let (sx, sy) = if contact > 0.5 {
        (1.32, 0.68)
    } else {
        let sy = 1.0 + 0.30 * speed * speed;
        (1.0 / sy, sy)
    };
    Pose { height, sx, sy }
}

fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn oval(cx: f32, cy: f32, rx: f32, ry: f32) -> Option<tiny_skia::Path> {
    PathBuilder::from_oval(Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0)?)
}

fn draw_frame(i: usize) -> Pixmap {
    let mut pm = Pixmap::new(W, H).expect("sample size");
    let pose = pose(i);
    let t = Transform::identity();
    let h01 = (pose.height / APEX).clamp(0.0, 1.0);

    // Shadow: wide and dark on the ground, shrinking and fading as the ball rises.
    let rx = RADIUS * (1.35 - 0.75 * h01);
    let alpha = (95.0 - 60.0 * h01) as u8;
    if let Some(p) = oval(CX, GROUND_Y + 8.0, rx, rx * 0.2) {
        pm.fill_path(&p, &paint([30, 30, 50, alpha]), FillRule::Winding, t, None);
    }

    // Ground line.
    if let Some(r) = Rect::from_xywh(110.0, GROUND_Y + 1.0, W as f32 - 220.0, 3.0) {
        pm.fill_path(
            &PathBuilder::from_rect(r),
            &paint([0x1B, 0x1B, 0x22, 255]),
            FillRule::Winding,
            t,
            None,
        );
    }

    // Ball anchored at its bottom so squashing keeps contact with the ground.
    let (rx, ry) = (RADIUS * pose.sx, RADIUS * pose.sy);
    let cy = GROUND_Y - pose.height - ry;
    if let Some(p) = oval(CX, cy, rx, ry) {
        pm.fill_path(&p, &paint([0xE8, 0x55, 0x3A, 255]), FillRule::Winding, t, None);
        // Highlight rides on the surface and deforms with the ball.
        if let Some(h) = oval(CX - rx * 0.32, cy - ry * 0.38, rx * 0.26, ry * 0.18) {
            pm.fill_path(&h, &paint([255, 190, 160, 210]), FillRule::Winding, t, None);
        }
        let stroke = Stroke {
            width: 5.0,
            ..Stroke::default()
        };
        let mut ink = Paint::default();
        ink.set_color_rgba8(0x1B, 0x1B, 0x22, 255);
        ink.anti_alias = true;
        pm.stroke_path(&p, &ink, &stroke, t, None);
    }
    pm
}

pub fn bouncing_ball() -> Project {
    let mut p = Project::empty("Bouncing ball", W, H);
    p.fps = 12;
    for i in 0..FRAMES {
        let f = p.frame_from_pixmap(draw_frame(i));
        let at = p.len();
        p.insert_frame(at, f);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_twelve_frames_at_12fps() {
        let p = bouncing_ball();
        assert_eq!(p.len(), 12);
        assert_eq!(p.fps, 12);
        assert!(p.frames().iter().all(|f| !f.is_blank()));
    }

    #[test]
    fn squashes_on_contact_stretches_near_ground_and_is_round_at_apex() {
        let contact = pose(0);
        assert_eq!(contact.height, 0.0);
        assert!(contact.sx > 1.2 && contact.sy < 0.8, "squash {contact:?}");

        let near_ground = pose(1);
        assert!(
            near_ground.sy > 1.1 && near_ground.sx < 0.95,
            "stretch {near_ground:?}"
        );
        let falling = pose(11);
        assert!(falling.sy > 1.1, "stretch while falling in {falling:?}");

        let apex = pose(6);
        assert!((apex.height - APEX).abs() < 1.0);
        assert!((apex.sy - 1.0).abs() < 0.01, "round at apex {apex:?}");
    }

    #[test]
    fn shadow_shrinks_with_height() {
        let shadow_px = |i: usize| {
            let f = draw_frame(i);
            let y = (GROUND_Y + 8.0) as u32;
            (0..W).filter(|&x| f.pixel(x, y).unwrap().alpha() > 10).count()
        };
        // Compare contact (frame 0, includes ball) against apex via the ground row far from ball.
        let wide = shadow_px(1);
        let narrow = shadow_px(6);
        assert!(narrow < wide, "{narrow} !< {wide}");
    }

    #[test]
    fn loop_is_continuous_between_last_and_first_frame() {
        // Ball is lowest at frame 0; frame 11 is close to the ground but not touching.
        assert!(pose(11).height > 0.0 && pose(11).height < 100.0);
        assert_eq!(pose(0).height, 0.0);
    }
}
