//! Bottom x-sheet strip: transport, onion/fps/hold controls, thumbnail cells.

use eframe::egui::{
    self, pos2, vec2, Align, Color32, CursorIcon, Frame, Layout, Margin, Panel, Rect, RichText, Sense, Stroke, Ui,
};

use super::theme::Palette;
use super::widgets::{self as w};
use crate::app::FlipLineApp;
use crate::model::timing::{self, FPS_PRESETS, MAX_HOLD, MIN_HOLD};
use crate::render;

const THUMB_H: f32 = 46.0;
const CELL_GAP: f32 = 4.0;
const LABEL_H: f32 = 16.0;

struct Cell {
    thumb: Option<egui::TextureHandle>,
    hold: u8,
}

pub fn show(app: &mut FlipLineApp, ui: &mut Ui) {
    let pal = Palette::of(ui.ctx());
    let ctx = ui.ctx().clone();
    Panel::bottom("timeline")
        .exact_size(150.0)
        .resizable(false)
        .frame(Frame::new().fill(pal.panel).inner_margin(Margin::symmetric(12, 8)).stroke(Stroke::new(1.0, pal.line)))
        .show_separator_line(false)
        .show(ui, |ui| {
            controls(app, ui, &ctx, &pal);
            ui.add_space(4.0);
            cells(app, ui, &ctx, &pal);
        });
}

fn controls(app: &mut FlipLineApp, ui: &mut Ui, ctx: &egui::Context, pal: &Palette) {
    let now = ctx.input(|i| i.time);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if w::icon_button(ui, 30.0, false, "Previous frame (, or ←)", w::icon_prev).clicked() {
            app.step_frame(-1);
        }
        let playing = app.playback.playing;
        let play_icon: w::Icon = if playing { w::icon_pause } else { w::icon_play };
        if w::icon_button(ui, 34.0, playing, if playing { "Pause (Space)" } else { "Play (Space)" }, play_icon).clicked() {
            app.toggle_playback(now);
        }
        if w::icon_button(ui, 30.0, false, "Next frame (. or →)", w::icon_next).clicked() {
            app.step_frame(1);
        }
        if w::icon_button(ui, 30.0, app.playback.looping, "Loop playback", w::icon_loop).clicked() {
            app.playback.looping = !app.playback.looping;
        }

        ui.add_space(6.0);
        for fps in FPS_PRESETS {
            if w::pill(ui, &fps.to_string(), app.project.fps == fps).on_hover_text(format!("{fps} frames per second")).clicked() {
                app.set_fps(fps, now);
            }
        }
        ui.label(RichText::new("fps").color(pal.dim));

        ui.add_space(10.0);
        if w::icon_button(ui, 30.0, app.onion.enabled, "Onion skin (O)", w::icon_onion).clicked() {
            app.onion.enabled = !app.onion.enabled;
        }
        for d in render::MIN_ONION_DEPTH..=render::MAX_ONION_DEPTH {
            if w::pill(ui, &d.to_string(), app.onion.enabled && app.onion.depth == d)
                .on_hover_text(format!("Onion skin depth: {d} frame{}", if d == 1 { "" } else { "s" }))
                .clicked()
            {
                app.onion.depth = d;
                app.onion.enabled = true;
            }
        }

        ui.add_space(10.0);
        if w::icon_button(ui, 30.0, false, "New blank frame (N)", w::icon_plus).clicked() {
            app.new_frame();
        }
        if w::icon_button(ui, 30.0, false, "Duplicate frame (D)", w::icon_duplicate).clicked() {
            app.duplicate_frame();
        }
        if w::icon_button(ui, 30.0, false, "Delete frame (Delete)", w::icon_trash).clicked() {
            app.delete_frame();
        }

        ui.add_space(10.0);
        let hold = app.project.frame(app.current).map(|f| f.hold).unwrap_or(1);
        if w::icon_button_en(ui, 26.0, false, hold > MIN_HOLD, "Shorter hold (-)", w::icon_minus).clicked() {
            app.change_hold(-1);
        }
        ui.label(RichText::new(format!("hold {hold}")).monospace());
        if w::icon_button_en(ui, 26.0, false, hold < MAX_HOLD, "Longer hold (=)", w::icon_plus).clicked() {
            app.change_hold(1);
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let holds = app.project.holds();
            let text = timing::timecode(&holds, app.current, app.project.fps);
            ui.label(RichText::new(text).monospace().size(13.0).color(pal.text));
        });
    });
}

fn cells(app: &mut FlipLineApp, ui: &mut Ui, ctx: &egui::Context, pal: &Palette) {
    let stroking_id = app.stroke.as_ref().and_then(|_| app.project.frame(app.current).map(|f| f.id));
    if render::update_thumbnails(ctx, &mut app.project, stroking_id, 6) {
        ctx.request_repaint();
    }

    let (pw, ph) = (app.project.width() as f32, app.project.height() as f32);
    let thumb_w = (THUMB_H * pw / ph).clamp(26.0, 96.0);
    let tick_w = (thumb_w + 12.0).max(58.0);
    let cell_h = THUMB_H + LABEL_H + 10.0;

    let cells: Vec<Cell> = app
        .project
        .frames()
        .iter()
        .map(|f| Cell { thumb: f.gpu.thumb.clone(), hold: f.hold })
        .collect();
    let n = cells.len();
    let current = app.current;
    let playing = app.playback.playing;
    let scroll_to = std::mem::take(&mut app.scroll_to_current);

    let mut pending_drag_from: Option<usize> = None;
    let mut click: Option<usize> = None;
    let mut rects: Vec<Rect> = Vec::with_capacity(n);
    let mut ctx_action: Option<(usize, Action)> = None;
    let drag_from = app.ui.drag_from;

    let area = egui::ScrollArea::horizontal().id_salt("cells").auto_shrink([false, false]);
    area.show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(CELL_GAP, 0.0);
        ui.horizontal(|ui| {
            for (i, c) in cells.iter().enumerate() {
                let w_ = tick_w * c.hold as f32;
                let (rect, resp) = ui.allocate_exact_size(vec2(w_, cell_h), Sense::click_and_drag());
                rects.push(rect);
                let active = i == current;
                let bg = if active { pal.accent_soft() } else { pal.cell };
                let border = if active { Stroke::new(2.0, pal.accent) } else { Stroke::new(1.0, pal.line) };
                let p = ui.painter();
                p.rect(rect, 6.0, bg, border, egui::StrokeKind::Inside);

                // Held span: lighter band with a continuation line and one tick per extra exposure.
                if c.hold > 1 {
                    let span = Rect::from_min_max(pos2(rect.left() + tick_w, rect.top() + 4.0), pos2(rect.right() - 4.0, rect.top() + 4.0 + THUMB_H + 2.0));
                    p.rect_filled(span, 4.0, pal.cell_hold);
                    let mid = span.center().y;
                    p.line_segment([pos2(span.left() - 2.0, mid), pos2(span.right() - 6.0, mid)], Stroke::new(1.5, if active { pal.accent } else { pal.dim }));
                    p.circle_filled(pos2(span.right() - 6.0, mid), 3.0, if active { pal.accent } else { pal.dim });
                    for k in 1..c.hold {
                        let x = rect.left() + tick_w * k as f32;
                        p.line_segment([pos2(x, rect.top() + 8.0), pos2(x, rect.top() + THUMB_H)], Stroke::new(1.0, pal.line));
                    }
                }

                let tr = Rect::from_min_size(pos2(rect.left() + 6.0, rect.top() + 5.0), vec2(thumb_w, THUMB_H));
                if let Some(t) = &c.thumb {
                    p.image(t.id(), tr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                p.rect_stroke(tr, 2.0, Stroke::new(1.0, pal.line), egui::StrokeKind::Outside);

                let num_color = if active { pal.accent } else { pal.dim };
                p.text(pos2(rect.left() + 8.0, rect.bottom() - 8.0), egui::Align2::LEFT_CENTER, format!("{:02}", i + 1), egui::FontId::monospace(12.0), num_color);
                if c.hold > 1 {
                    p.text(pos2(rect.right() - 8.0, rect.bottom() - 8.0), egui::Align2::RIGHT_CENTER, format!("×{}", c.hold), egui::FontId::monospace(11.5), num_color);
                }
                if playing && active {
                    p.rect_filled(Rect::from_min_size(rect.left_top(), vec2(w_, 3.0)), 2.0, pal.playhead);
                }

                if resp.clicked() {
                    click = Some(i);
                }
                if resp.drag_started() {
                    pending_drag_from = Some(i);
                    click = Some(i);
                }
                if resp.dragged() {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                resp.context_menu(|ui| {
                    if ui.button("New frame after").clicked() {
                        ctx_action = Some((i, Action::New));
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        ctx_action = Some((i, Action::Duplicate));
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        ctx_action = Some((i, Action::Delete));
                        ui.close();
                    }
                });
                if scroll_to && active {
                    ui.scroll_to_rect(rect.expand2(vec2(30.0, 0.0)), Some(Align::Center));
                }
            }
        });
    });

    if let Some(i) = pending_drag_from {
        app.ui.drag_from = Some(i);
    }
    if let Some(i) = click {
        app.stop_playback();
        app.set_current(i);
    }
    if let Some((i, a)) = ctx_action {
        app.set_current(i);
        match a {
            Action::New => app.new_frame(),
            Action::Duplicate => app.duplicate_frame(),
            Action::Delete => app.delete_frame(),
        }
    }

    // Drag-to-reorder: show the insertion gap while dragging, commit on release.
    if let Some(from) = drag_from.or(app.ui.drag_from) {
        let pos = ctx.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos()));
        if let (Some(pos), true) = (pos, !rects.is_empty()) {
            let gap = rects.iter().take_while(|r| r.center().x < pos.x).count();
            let x = match rects.get(gap) {
                Some(r) => r.left() - CELL_GAP * 0.5,
                None => rects.last().map(|r| r.right() + CELL_GAP * 0.5).unwrap_or(0.0),
            };
            let top = rects[0].top();
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drag_marker"));
            let p = ctx.layer_painter(layer);
            p.line_segment([pos2(x, top), pos2(x, top + cell_h)], Stroke::new(3.0, pal.accent));
            if !ctx.input(|i| i.pointer.primary_down()) {
                let target = if gap > from { gap - 1 } else { gap };
                app.ui.drag_from = None;
                app.move_frame_to(from, target.min(n.saturating_sub(1)));
            }
        } else if !ctx.input(|i| i.pointer.primary_down()) {
            app.ui.drag_from = None;
        }
    }
}

enum Action {
    New,
    Duplicate,
    Delete,
}
