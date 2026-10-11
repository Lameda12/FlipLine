//! Centre panel: paper, peg bar, onion skin, the current frame, and pointer input for tools,
//! zoom and pan.

use eframe::egui::{
    self, pos2, vec2, Color32, CursorIcon, Event, Frame, Order, Painter, PointerButton, Pos2, Rect, Sense,
    Shape, Stroke, TouchPhase, Ui, Vec2,
};

use super::theme::Palette;
use crate::app::{FlipLineApp, ZoomRequest};
use crate::render::{self, color32};
use crate::tools::{Sample, ToolId};

const PEG_RESERVE: f32 = 46.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 32.0;
const UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

pub fn show(app: &mut FlipLineApp, ui: &mut Ui) {
    let pal = Palette::of(ui.ctx());
    egui::CentralPanel::default()
        .frame(Frame::new().fill(pal.desk))
        .show(ui, |ui| {
            let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
            let ctx = ui.ctx().clone();
            let ppp = ctx.pixels_per_point();
            let (cw, ch) = (app.project.width() as f32, app.project.height() as f32);

            // ---- view: fit / zoom / pan ------------------------------------------------------
            let base = rect.center() + vec2(0.0, PEG_RESERVE * 0.5);
            let fit_zoom = (((rect.width() - 64.0) / cw).min((rect.height() - PEG_RESERVE - 40.0) / ch)
                * ppp)
                .clamp(MIN_ZOOM, MAX_ZOOM);
            if let Some(req) = app.view.request.take() {
                match req {
                    ZoomRequest::Fit => app.view.fit = true,
                    ZoomRequest::Actual => zoom_to(app, 1.0, base, base, ppp),
                    ZoomRequest::By(f) => {
                        let z = app.view.zoom * f;
                        zoom_to(app, z, base, base, ppp);
                    }
                }
            }
            if app.view.fit {
                app.view.zoom = fit_zoom;
                app.view.pan = Vec2::ZERO;
            }

            let pointer = ctx.input(|i| i.pointer.hover_pos());
            if resp.hovered() {
                let (zd, scroll) = ctx.input(|i| (i.zoom_delta(), i.smooth_scroll_delta()));
                if zd != 1.0 {
                    let at = pointer.unwrap_or(base);
                    let z = app.view.zoom * zd;
                    zoom_to(app, z, at, base, ppp);
                } else if scroll != Vec2::ZERO {
                    app.view.pan += scroll;
                    app.view.fit = false;
                }
            }
            if let Some(mt) = ctx.input(|i| i.multi_touch()) {
                // Two-finger gesture: pan, never draw.
                app.cancel_stroke();
                app.view.pan += mt.translation_delta;
                app.view.fit = false;
            }
            let space_pan = app.input.space_down && resp.dragged_by(PointerButton::Primary);
            if resp.dragged_by(PointerButton::Middle) || space_pan {
                app.view.pan += resp.drag_delta();
                app.view.fit = false;
                app.input.space_used = true;
            }
            app.input.panning = resp.dragged_by(PointerButton::Middle) || space_pan;

            let s = app.view.zoom / ppp; // points per canvas pixel
            let centre = base + app.view.pan;
            let canvas = Rect::from_center_size(centre, vec2(cw * s, ch * s));

            // ---- pointer input -----------------------------------------------------------------
            let layer = ui.layer_id();
            let sample_at = |pos: Pos2, pressure: Option<f32>| {
                let p = (pos - canvas.min) / s;
                Sample::new(p.x, p.y, pressure.unwrap_or(1.0))
            };
            let events: Vec<Event> = ctx.input(|i| i.events.clone());
            let multi = ctx.input(|i| i.multi_touch()).is_some();
            for ev in events {
                match ev {
                    Event::Touch { phase, force, .. } => match phase {
                        TouchPhase::Start | TouchPhase::Move => {
                            app.input.pressure = force.filter(|f| *f > 0.0 && *f <= 1.0);
                            if app.input.pressure.is_some() {
                                app.input.saw_pressure = true;
                            }
                        }
                        TouchPhase::End | TouchPhase::Cancel => app.input.pressure = None,
                    },
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: true,
                        ..
                    } => {
                        let over_canvas_layer = ctx
                            .layer_id_at(pos)
                            .is_none_or(|l| l.order == Order::Background || l == layer);
                        if rect.contains(pos)
                            && over_canvas_layer
                            && !app.input.space_down
                            && !multi
                            && !app.playback.playing
                            && !blocked_by_dialog(app)
                        {
                            app.pointer_down(sample_at(pos, app.input.pressure));
                        }
                    }
                    Event::PointerMoved(pos) if app.stroke.is_some() => {
                        app.pointer_drag(sample_at(pos, app.input.pressure));
                    }
                    Event::PointerButton {
                        button: PointerButton::Primary,
                        pressed: false,
                        ..
                    } => {
                        app.pointer_up();
                    }
                    _ => {}
                }
            }
            if app.stroke.is_some() && !ctx.input(|i| i.pointer.primary_down()) {
                app.pointer_up();
            }

            // ---- paint ---------------------------------------------------------------------------
            let painter = ui.painter_at(rect);
            draw_peg_bar(&painter, canvas, s, &pal);
            let shadow = egui::Shadow {
                offset: [0, 5],
                blur: 22,
                spread: 0,
                color: Color32::from_black_alpha(if pal.dark { 120 } else { 55 }),
            };
            painter.add(shadow.as_shape(canvas, 2.0));
            painter.rect_filled(canvas, 2.0, color32(app.project.paper));

            let onion_tex = if app.onion.enabled && !app.playback.playing {
                app.onion_cache
                    .get(&ctx, &app.project, app.current, app.onion.depth)
            } else {
                None
            };
            if let Some(t) = onion_tex {
                painter.image(t.id(), canvas, UV, Color32::WHITE);
            }
            if let Some(frame) = app.project.frame_mut(app.current) {
                if let Some(t) = render::sync_texture(&ctx, frame) {
                    painter.image(t.id(), canvas, UV, Color32::WHITE);
                }
            }
            painter.rect_stroke(canvas, 2.0, Stroke::new(1.0, pal.line), egui::StrokeKind::Outside);

            // ---- cursor ----------------------------------------------------------------------------
            if resp.hovered() {
                if app.input.space_down || app.input.panning {
                    ctx.set_cursor_icon(if app.input.panning {
                        CursorIcon::Grabbing
                    } else {
                        CursorIcon::Grab
                    });
                } else if !app.playback.playing {
                    match app.tool {
                        ToolId::Brush | ToolId::Eraser => {
                            if let Some(p) = pointer.filter(|p| rect.contains(*p)) {
                                ctx.set_cursor_icon(CursorIcon::None);
                                let r = (app.settings.size * 0.5 * s).max(2.0);
                                draw_brush_ring(&painter, p, r);
                            }
                        }
                        ToolId::Fill | ToolId::Picker => ctx.set_cursor_icon(CursorIcon::Crosshair),
                    }
                }
            }

            if app.ui.drop_hover {
                painter.rect_filled(rect, 0.0, pal.accent.gamma_multiply(0.12));
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop images or a .flipline file",
                    egui::FontId::proportional(20.0),
                    pal.accent,
                );
            }
        });
}

fn blocked_by_dialog(app: &FlipLineApp) -> bool {
    app.ui.new_dialog.is_some() || app.ui.export_dialog.is_some() || app.ui.confirm.is_some() || app.ui.help
}

/// Zooms to `zoom` keeping the canvas point under `anchor` fixed.
fn zoom_to(app: &mut FlipLineApp, zoom: f32, anchor: Pos2, base: Pos2, ppp: f32) {
    let z = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    let factor = z / app.view.zoom;
    let d = anchor - base;
    app.view.pan = d - (d - app.view.pan) * factor;
    app.view.zoom = z;
    app.view.fit = false;
    let _ = ppp;
}

fn draw_peg_bar(p: &Painter, canvas: Rect, s: f32, pal: &Palette) {
    let h = (24.0 * s).clamp(14.0, 30.0);
    let w = (canvas.width() * 0.5).clamp(120.0, 460.0).min(canvas.width());
    let bar = Rect::from_center_size(pos2(canvas.center().x, canvas.top() - 8.0 - h * 0.5), vec2(w, h));
    p.rect_filled(
        bar.translate(vec2(0.0, 2.5)),
        4.0,
        Color32::from_black_alpha(if pal.dark { 90 } else { 40 }),
    );
    p.rect_filled(bar, 4.0, pal.metal);
    p.rect_stroke(
        bar,
        4.0,
        Stroke::new(1.0, pal.metal_dark),
        egui::StrokeKind::Inside,
    );
    p.line_segment(
        [bar.left_top() + vec2(4.0, 2.0), bar.right_top() + vec2(-4.0, 2.0)],
        Stroke::new(1.0, Color32::from_white_alpha(90)),
    );
    // One round centre peg flanked by two slotted pegs, like a real animation disc.
    let c = bar.center();
    let r = h * 0.28;
    p.circle_filled(c, r, pal.metal_dark);
    p.circle_stroke(c, r, Stroke::new(1.0, Color32::from_white_alpha(70)));
    for side in [-1.0, 1.0] {
        let slot = Rect::from_center_size(pos2(c.x + side * w * 0.33, c.y), vec2(h * 0.95, r * 1.5));
        p.rect_filled(slot, r * 0.75, pal.metal_dark);
        p.rect_stroke(
            slot,
            r * 0.75,
            Stroke::new(1.0, Color32::from_white_alpha(70)),
            egui::StrokeKind::Inside,
        );
    }
}

fn draw_brush_ring(p: &Painter, centre: Pos2, r: f32) {
    p.circle_stroke(centre, r + 0.5, Stroke::new(2.0, Color32::from_white_alpha(210)));
    p.circle_stroke(centre, r, Stroke::new(1.0, Color32::from_black_alpha(220)));
    p.add(Shape::circle_filled(centre, 1.2, Color32::from_black_alpha(220)));
}
