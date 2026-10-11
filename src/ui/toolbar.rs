//! Left tool rail: tools, ink swatches, size, fill tolerance.

use eframe::egui::{self, Frame, Margin, Panel, RichText, Slider, Ui};

use super::theme::Palette;
use super::widgets::{self as w};
use crate::app::{FlipLineApp, SWATCHES};
use crate::render::color32;
use crate::tools::{ToolId, MAX_SIZE, MIN_SIZE};

fn tool_icon(t: ToolId) -> w::Icon {
    match t {
        ToolId::Brush => w::icon_brush,
        ToolId::Eraser => w::icon_eraser,
        ToolId::Fill => w::icon_fill,
        ToolId::Picker => w::icon_picker,
    }
}

pub fn show(app: &mut FlipLineApp, ui: &mut Ui) {
    let pal = Palette::of(ui.ctx());
    Panel::left("tool_rail")
        .exact_size(92.0)
        .resizable(false)
        .frame(Frame::new().fill(pal.rail).inner_margin(Margin::symmetric(10, 12)))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            w::label_caps(ui, "Tools");
            egui::Grid::new("tools").spacing([6.0, 6.0]).show(ui, |ui| {
                for (i, t) in ToolId::ALL.into_iter().enumerate() {
                    let tip = format!("{} ({})", t.name(), t.key());
                    if w::icon_button(ui, 34.0, app.tool == t, &tip, tool_icon(t)).clicked() {
                        app.set_tool(t);
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });

            ui.add_space(8.0);
            w::label_caps(ui, "Ink");
            egui::Grid::new("swatches").spacing([6.0, 6.0]).show(ui, |ui| {
                for (i, c) in SWATCHES.iter().enumerate() {
                    if w::swatch(ui, color32(*c), app.settings.color == *c, 34.0).clicked() {
                        app.settings.color = *c;
                        if app.tool == ToolId::Picker {
                            app.set_tool(app.return_tool);
                        }
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            w::label_caps(ui, "Custom");
            let before = app.settings.color;
            let mut c = before;
            ui.spacing_mut().interact_size.x = 74.0;
            ui.color_edit_button_srgb(&mut c).on_hover_text("Custom colour");
            if c != before {
                app.settings.color = c;
            }

            ui.add_space(8.0);
            w::label_caps(ui, "Size");
            ui.spacing_mut().slider_width = 70.0;
            ui.add(Slider::new(&mut app.settings.size, MIN_SIZE..=MAX_SIZE).show_value(false).step_by(1.0));
            ui.label(RichText::new(format!("{:.0} px", app.settings.size)).monospace().color(pal.dim));
            // Live preview of the stroke width.
            let (r, _) = ui.allocate_exact_size(egui::vec2(70.0, 26.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 6.0, pal.cell);
            ui.painter().circle_filled(
                r.center(),
                (app.settings.size * 0.5).clamp(0.6, 12.5),
                if app.tool == ToolId::Eraser { pal.dim } else { color32(app.settings.color) },
            );

            if app.tool == ToolId::Fill {
                ui.add_space(8.0);
                w::label_caps(ui, "Tolerance");
                let mut t = app.settings.tolerance as f32;
                if ui.add(Slider::new(&mut t, 0.0..=128.0).show_value(false)).changed() {
                    app.settings.tolerance = t as u8;
                }
                ui.label(RichText::new(format!("{}", app.settings.tolerance)).monospace().color(pal.dim));
            }

            ui.add_space(8.0);
            let pressure = if app.input.saw_pressure {
                "Pen pressure: on"
            } else {
                "Pressure: waiting for a pen"
            };
            ui.label(RichText::new(pressure).size(10.5).color(pal.dim));
        });
}
