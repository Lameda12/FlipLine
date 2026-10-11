use eframe::egui::{self, Align, Frame, Layout, Margin, Panel, RichText, Ui};

use super::theme::Palette;
use super::widgets::{self as w};
use crate::app::{FlipLineApp, ZoomRequest};

pub fn show(app: &mut FlipLineApp, ui: &mut Ui) {
    let pal = Palette::of(ui.ctx());
    let ctx = ui.ctx().clone();
    Panel::top("topbar")
        .frame(
            Frame::new()
                .fill(pal.panel)
                .inner_margin(Margin::symmetric(12, 7))
                .stroke(egui::Stroke::new(1.0, pal.line)),
        )
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("FlipLine").strong().size(17.0).color(pal.accent));
                ui.add_space(4.0);

                let mut name = app.project.name.clone();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(170.0)
                        .background_color(egui::Color32::TRANSPARENT)
                        .font(egui::TextStyle::Heading)
                        .hint_text("Untitled"),
                );
                if resp.changed() {
                    app.project.name = name;
                    app.mark_modified();
                }
                if app.is_modified() {
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.5, pal.accent);
                    resp.on_hover_text("Unsaved changes");
                }

                ui.add_space(10.0);
                if w::icon_button_en(
                    ui,
                    30.0,
                    false,
                    app.history.can_undo(),
                    "Undo (Ctrl/Cmd+Z)",
                    w::icon_undo,
                )
                .clicked()
                {
                    app.undo();
                }
                if w::icon_button_en(
                    ui,
                    30.0,
                    false,
                    app.history.can_redo(),
                    "Redo (Shift+Ctrl/Cmd+Z)",
                    w::icon_redo,
                )
                .clicked()
                {
                    app.redo();
                }
                ui.add_space(10.0);

                if w::pill(ui, "New", false).on_hover_text("New project").clicked() {
                    app.ui.new_dialog = Some(crate::app::NewProjectDialog {
                        name: "Untitled".into(),
                        size_idx: crate::model::project::SIZE_PRESETS
                            .iter()
                            .position(|&(_, w_, h_)| w_ == app.project.width() && h_ == app.project.height())
                            .unwrap_or(0),
                    });
                }
                if w::pill(ui, "Open", false)
                    .on_hover_text("Open a .flipline project")
                    .clicked()
                {
                    crate::platform::pick_project(app.sender(), ctx.clone());
                }
                if w::pill(ui, "Import", false)
                    .on_hover_text("Import images (or drop a numbered sequence on the window)")
                    .clicked()
                {
                    crate::platform::pick_images(app.sender(), ctx.clone());
                }
                if w::pill(ui, "Save", false)
                    .on_hover_text("Save (Ctrl/Cmd+S)")
                    .clicked()
                {
                    app.save(&ctx, false);
                }
                if w::accent_button(ui, "Export")
                    .on_hover_text("Export (Ctrl/Cmd+E)")
                    .clicked()
                {
                    app.open_export_dialog();
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if w::icon_button(ui, 30.0, app.ui.help, "Shortcuts (?)", w::icon_help).clicked() {
                        app.ui.help = !app.ui.help;
                    }
                    ui.add_space(6.0);
                    if w::icon_button(ui, 28.0, false, "Zoom in", w::icon_plus).clicked() {
                        app.view.request = Some(ZoomRequest::By(1.25));
                    }
                    let pct = format!("{:.0}%", app.view.zoom * 100.0);
                    let r = ui.add(
                        egui::Label::new(RichText::new(pct).monospace().color(pal.dim))
                            .sense(egui::Sense::click()),
                    );
                    if r.on_hover_text("Click for 100%").clicked() {
                        app.view.request = Some(ZoomRequest::Actual);
                    }
                    if w::icon_button(ui, 28.0, false, "Zoom out", w::icon_minus).clicked() {
                        app.view.request = Some(ZoomRequest::By(0.8));
                    }
                    if w::icon_button(ui, 28.0, app.view.fit, "Fit to window (F)", w::icon_fit).clicked() {
                        app.view.request = Some(ZoomRequest::Fit);
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("{}×{}", app.project.width(), app.project.height()))
                            .monospace()
                            .color(pal.dim),
                    );
                });
            });
        });
}
