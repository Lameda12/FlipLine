use eframe::egui::{self, Align2, Context, Id, Order, RichText};

use super::theme::Palette;

struct Toast {
    text: String,
    error: bool,
    born: Option<f64>,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub fn info(&mut self, text: impl Into<String>) {
        self.items.push(Toast {
            text: text.into(),
            error: false,
            born: None,
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::warn!("{text}");
        self.items.push(Toast {
            text,
            error: true,
            born: None,
        });
    }

    pub fn show(&mut self, ctx: &Context) {
        if self.items.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        for t in &mut self.items {
            t.born.get_or_insert(now);
        }
        self.items
            .retain(|t| now - t.born.unwrap_or(now) < if t.error { 8.0 } else { 3.5 });
        let pal = Palette::of(ctx);
        egui::Area::new(Id::new("toasts"))
            .anchor(Align2::RIGHT_TOP, egui::vec2(-16.0, 62.0))
            .order(Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for t in self.items.iter().rev().take(4) {
                    let accent = if t.error { pal.playhead } else { pal.accent };
                    egui::Frame::new()
                        .fill(pal.panel)
                        .stroke(egui::Stroke::new(1.0, pal.line))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .shadow(egui::Shadow {
                            offset: [0, 3],
                            blur: 12,
                            spread: 0,
                            color: egui::Color32::from_black_alpha(60),
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(egui::vec2(4.0, 16.0), egui::Sense::hover());
                                ui.painter().rect_filled(r, 2.0, accent);
                                ui.label(RichText::new(&t.text).color(pal.text));
                            });
                        });
                    ui.add_space(4.0);
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
