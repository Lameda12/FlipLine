//! Small custom-painted controls and icons (no icon font dependency, crisp at any DPI).

use eframe::egui::{
    self, pos2, vec2, Color32, CursorIcon, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
};

use super::theme::Palette;

pub type Icon = fn(&Painter, Rect, Color32);

/// Square icon button. `selected` draws the accent state used for the active tool/toggle.
pub fn icon_button(ui: &mut Ui, size: f32, selected: bool, tip: &str, icon: Icon) -> Response {
    icon_button_en(ui, size, selected, true, tip, icon)
}

pub fn icon_button_en(ui: &mut Ui, size: f32, selected: bool, enabled: bool, tip: &str, icon: Icon) -> Response {
    let pal = Palette::of(ui.ctx());
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), sense);
    let hovered = resp.hovered() && enabled;
    let bg = if selected {
        pal.accent_soft()
    } else if hovered {
        pal.cell_hold
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if selected {
        Stroke::new(1.0, pal.accent)
    } else {
        Stroke::NONE
    };
    ui.painter().rect(rect, 7.0, bg, stroke, egui::StrokeKind::Inside);
    let fg = if !enabled {
        pal.dim.gamma_multiply(0.45)
    } else if selected {
        pal.accent
    } else {
        pal.text
    };
    icon(ui.painter(), rect.shrink(size * 0.22), fg);
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
    } else {
        resp.on_hover_text(tip)
    }
}

/// Text button that highlights when `selected`.
pub fn pill(ui: &mut Ui, text: &str, selected: bool) -> Response {
    let pal = Palette::of(ui.ctx());
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        if selected { pal.accent } else { pal.text },
    );
    let size = galley.size() + vec2(18.0, 8.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if selected {
        pal.accent_soft()
    } else if resp.hovered() {
        pal.cell_hold
    } else {
        pal.rail
    };
    let stroke = Stroke::new(1.0, if selected { pal.accent } else { pal.line });
    ui.painter().rect(rect, 7.0, bg, stroke, egui::StrokeKind::Inside);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, pal.text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn accent_button(ui: &mut Ui, text: &str) -> Response {
    let pal = Palette::of(ui.ctx());
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        pal.accent_fg,
    );
    let size = galley.size() + vec2(24.0, 10.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if resp.hovered() { pal.accent.gamma_multiply(0.9) } else { pal.accent };
    ui.painter().rect_filled(rect, 7.0, bg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, pal.accent_fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn label_caps(ui: &mut Ui, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.label(egui::RichText::new(text.to_uppercase()).size(10.5).color(pal.dim).strong());
}

fn p(r: Rect, x: f32, y: f32) -> Pos2 {
    pos2(r.left() + r.width() * x, r.top() + r.height() * y)
}

fn line(pt: &Painter, pts: &[Pos2], w: f32, c: Color32) {
    pt.add(Shape::line(pts.to_vec(), Stroke::new(w, c)));
}

fn poly(pt: &Painter, pts: &[Pos2], fill: Color32, stroke: Stroke) {
    pt.add(Shape::convex_polygon(pts.to_vec(), fill, stroke));
}

// ---- tool icons -------------------------------------------------------------------------

pub fn icon_brush(pt: &Painter, r: Rect, c: Color32) {
    line(pt, &[p(r, 0.95, 0.05), p(r, 0.38, 0.62)], 2.6, c);
    poly(
        pt,
        &[p(r, 0.42, 0.52), p(r, 0.55, 0.68), p(r, 0.30, 0.80), p(r, 0.08, 0.95), p(r, 0.12, 0.62)],
        c,
        Stroke::NONE,
    );
}

pub fn icon_eraser(pt: &Painter, r: Rect, c: Color32) {
    let body = [p(r, 0.08, 0.62), p(r, 0.52, 0.12), p(r, 0.98, 0.52), p(r, 0.55, 0.98)];
    poly(pt, &body, Color32::TRANSPARENT, Stroke::new(1.8, c));
    poly(pt, &[p(r, 0.30, 0.37), p(r, 0.75, 0.75), p(r, 0.55, 0.98), p(r, 0.08, 0.62)], c.gamma_multiply(0.55), Stroke::NONE);
    line(pt, &[p(r, 0.45, 1.0), p(r, 1.0, 1.0)], 1.8, c);
}

pub fn icon_fill(pt: &Painter, r: Rect, c: Color32) {
    let tilt = [p(r, 0.12, 0.45), p(r, 0.48, 0.08), p(r, 0.88, 0.50), p(r, 0.50, 0.88)];
    poly(pt, &tilt, c.gamma_multiply(0.35), Stroke::new(1.8, c));
    line(pt, &[p(r, 0.30, 0.20), p(r, 0.48, 0.02), p(r, 0.62, 0.14)], 1.8, c);
    pt.circle_filled(p(r, 0.93, 0.78), r.width() * 0.09, c);
    poly(pt, &[p(r, 0.93, 0.56), p(r, 0.86, 0.72), p(r, 1.0, 0.72)], c, Stroke::NONE);
}

pub fn icon_picker(pt: &Painter, r: Rect, c: Color32) {
    line(pt, &[p(r, 0.18, 0.88), p(r, 0.62, 0.44)], 2.4, c);
    poly(pt, &[p(r, 0.52, 0.32), p(r, 0.68, 0.48), p(r, 0.30, 0.86), p(r, 0.10, 0.92), p(r, 0.14, 0.72)], c.gamma_multiply(0.35), Stroke::new(1.6, c));
    pt.circle_filled(p(r, 0.78, 0.22), r.width() * 0.17, c);
}

// ---- action icons -----------------------------------------------------------------------

fn arc(pt: &Painter, r: Rect, flip: bool, c: Color32) {
    let ctr = p(r, 0.5, 0.58);
    let rad = r.width() * 0.38;
    let mut pts = Vec::new();
    for i in 0..=14 {
        let a = -std::f32::consts::PI * 0.15 - (i as f32 / 14.0) * std::f32::consts::PI * 1.05;
        let a = if flip { std::f32::consts::PI - a } else { a };
        pts.push(ctr + vec2(a.cos(), a.sin()) * rad);
    }
    line(pt, &pts, 2.2, c);
    let tip = pts[14];
    let dir = if flip { 1.0 } else { -1.0 };
    poly(pt, &[tip + vec2(dir * -1.0, -5.5), tip + vec2(dir * 6.5, 1.0), tip + vec2(dir * -1.0, 7.0)], c, Stroke::NONE);
}

pub fn icon_undo(pt: &Painter, r: Rect, c: Color32) {
    arc(pt, r, false, c);
}

pub fn icon_redo(pt: &Painter, r: Rect, c: Color32) {
    arc(pt, r, true, c);
}

pub fn icon_play(pt: &Painter, r: Rect, c: Color32) {
    poly(pt, &[p(r, 0.18, 0.02), p(r, 0.98, 0.5), p(r, 0.18, 0.98)], c, Stroke::NONE);
}

pub fn icon_pause(pt: &Painter, r: Rect, c: Color32) {
    pt.rect_filled(Rect::from_min_max(p(r, 0.12, 0.05), p(r, 0.40, 0.95)), 1.5, c);
    pt.rect_filled(Rect::from_min_max(p(r, 0.60, 0.05), p(r, 0.88, 0.95)), 1.5, c);
}

pub fn icon_prev(pt: &Painter, r: Rect, c: Color32) {
    pt.rect_filled(Rect::from_min_max(p(r, 0.08, 0.1), p(r, 0.22, 0.9)), 1.0, c);
    poly(pt, &[p(r, 0.95, 0.1), p(r, 0.30, 0.5), p(r, 0.95, 0.9)], c, Stroke::NONE);
}

pub fn icon_next(pt: &Painter, r: Rect, c: Color32) {
    pt.rect_filled(Rect::from_min_max(p(r, 0.78, 0.1), p(r, 0.92, 0.9)), 1.0, c);
    poly(pt, &[p(r, 0.05, 0.1), p(r, 0.70, 0.5), p(r, 0.05, 0.9)], c, Stroke::NONE);
}

pub fn icon_plus(pt: &Painter, r: Rect, c: Color32) {
    line(pt, &[p(r, 0.5, 0.05), p(r, 0.5, 0.95)], 2.2, c);
    line(pt, &[p(r, 0.05, 0.5), p(r, 0.95, 0.5)], 2.2, c);
}

pub fn icon_minus(pt: &Painter, r: Rect, c: Color32) {
    line(pt, &[p(r, 0.1, 0.5), p(r, 0.9, 0.5)], 2.2, c);
}

pub fn icon_duplicate(pt: &Painter, r: Rect, c: Color32) {
    pt.rect_stroke(Rect::from_min_max(p(r, 0.30, 0.02), p(r, 0.98, 0.70)), 2.0, Stroke::new(1.8, c), egui::StrokeKind::Inside);
    pt.rect_filled(Rect::from_min_max(p(r, 0.02, 0.30), p(r, 0.70, 0.98)), 2.0, c.gamma_multiply(0.45));
    pt.rect_stroke(Rect::from_min_max(p(r, 0.02, 0.30), p(r, 0.70, 0.98)), 2.0, Stroke::new(1.8, c), egui::StrokeKind::Inside);
}

pub fn icon_trash(pt: &Painter, r: Rect, c: Color32) {
    line(pt, &[p(r, 0.08, 0.20), p(r, 0.92, 0.20)], 2.0, c);
    line(pt, &[p(r, 0.36, 0.20), p(r, 0.36, 0.05), p(r, 0.64, 0.05), p(r, 0.64, 0.20)], 1.8, c);
    line(pt, &[p(r, 0.18, 0.28), p(r, 0.26, 0.97), p(r, 0.74, 0.97), p(r, 0.82, 0.28)], 1.8, c);
    line(pt, &[p(r, 0.5, 0.40), p(r, 0.5, 0.82)], 1.6, c);
}

pub fn icon_onion(pt: &Painter, r: Rect, c: Color32) {
    pt.circle_stroke(p(r, 0.36, 0.5), r.width() * 0.34, Stroke::new(1.8, Palette::new(true).playhead.gamma_multiply(if c.r() > 200 { 1.0 } else { 0.9 })));
    pt.circle_stroke(p(r, 0.64, 0.5), r.width() * 0.34, Stroke::new(1.8, Color32::from_rgb(0x2E, 0xA8, 0x5B)));
    pt.circle_filled(p(r, 0.5, 0.5), r.width() * 0.07, c);
}

pub fn icon_loop(pt: &Painter, r: Rect, c: Color32) {
    let ctr = p(r, 0.5, 0.5);
    let rad = r.width() * 0.38;
    for (start, end) in [(0.3_f32, 2.7_f32), (3.44, 5.84)] {
        let n = 12;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = start + (end - start) * i as f32 / n as f32;
                ctr + vec2(a.cos(), a.sin()) * rad
            })
            .collect();
        line(pt, &pts, 2.0, c);
        let tip = *pts.last().unwrap();
        let a = end + std::f32::consts::FRAC_PI_2;
        let d = vec2(a.cos(), a.sin());
        let n_ = vec2(-d.y, d.x);
        poly(pt, &[tip + d * 5.0, tip + n_ * 4.0, tip - n_ * 4.0], c, Stroke::NONE);
    }
}

pub fn icon_fit(pt: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.8, c);
    for (cx, cy, dx, dy) in [(0.0, 0.0, 1.0, 1.0), (1.0, 0.0, -1.0, 1.0), (0.0, 1.0, 1.0, -1.0), (1.0, 1.0, -1.0, -1.0)] {
        let corner = p(r, cx, cy);
        pt.add(Shape::line(
            vec![corner + vec2(dx * r.width() * 0.38, 0.0), corner, corner + vec2(0.0, dy * r.height() * 0.38)],
            s,
        ));
    }
}

pub fn icon_help(pt: &Painter, r: Rect, c: Color32) {
    pt.text(r.center(), egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(r.height() * 1.15), c);
}

pub fn icon_new(pt: &Painter, r: Rect, c: Color32) {
    poly(pt, &[p(r, 0.15, 0.02), p(r, 0.62, 0.02), p(r, 0.88, 0.28), p(r, 0.88, 0.98), p(r, 0.15, 0.98)], Color32::TRANSPARENT, Stroke::new(1.8, c));
    line(pt, &[p(r, 0.62, 0.02), p(r, 0.62, 0.28), p(r, 0.88, 0.28)], 1.6, c);
}

pub fn icon_open(pt: &Painter, r: Rect, c: Color32) {
    poly(pt, &[p(r, 0.02, 0.2), p(r, 0.38, 0.2), p(r, 0.5, 0.34), p(r, 0.92, 0.34), p(r, 0.92, 0.88), p(r, 0.02, 0.88)], Color32::TRANSPARENT, Stroke::new(1.8, c));
}

pub fn icon_save(pt: &Painter, r: Rect, c: Color32) {
    poly(pt, &[p(r, 0.08, 0.08), p(r, 0.82, 0.08), p(r, 0.94, 0.20), p(r, 0.94, 0.94), p(r, 0.08, 0.94)], Color32::TRANSPARENT, Stroke::new(1.8, c));
    pt.rect_stroke(Rect::from_min_max(p(r, 0.28, 0.08), p(r, 0.72, 0.38)), 1.0, Stroke::new(1.6, c), egui::StrokeKind::Inside);
    pt.rect_stroke(Rect::from_min_max(p(r, 0.24, 0.58), p(r, 0.76, 0.94)), 1.0, Stroke::new(1.6, c), egui::StrokeKind::Inside);
}

pub fn icon_export(pt: &Painter, r: Rect, c: Color32) {
    line(pt, &[p(r, 0.5, 0.62), p(r, 0.5, 0.04)], 2.0, c);
    poly(pt, &[p(r, 0.28, 0.24), p(r, 0.5, 0.0), p(r, 0.72, 0.24)], c, Stroke::NONE);
    line(pt, &[p(r, 0.12, 0.45), p(r, 0.12, 0.96), p(r, 0.88, 0.96), p(r, 0.88, 0.45)], 1.8, c);
}

/// Draws a swatch chip; returns the response.
pub fn swatch(ui: &mut Ui, color: Color32, selected: bool, size: f32) -> Response {
    let pal = Palette::of(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    ui.painter().rect_filled(rect, 6.0, color);
    let stroke = if selected {
        Stroke::new(2.5, pal.accent)
    } else if resp.hovered() {
        Stroke::new(1.5, pal.dim)
    } else {
        Stroke::new(1.0, pal.line)
    };
    ui.painter().rect_stroke(rect, 6.0, stroke, egui::StrokeKind::Inside);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}
