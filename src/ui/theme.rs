//! The "light table" look: warm paper-and-graphite neutrals with one orange accent, in light and
//! dark variants that follow the OS theme.

use eframe::egui::{
    self, Color32, Context, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme,
    Visuals,
};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// The desk surrounding the paper.
    pub desk: Color32,
    pub panel: Color32,
    pub rail: Color32,
    pub line: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_fg: Color32,
    pub cell: Color32,
    pub cell_hold: Color32,
    pub playhead: Color32,
    pub metal: Color32,
    pub metal_dark: Color32,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        let c = |r, g, b| Color32::from_rgb(r, g, b);
        if dark {
            Self {
                dark,
                desk: c(0x14, 0x15, 0x16),
                panel: c(0x1C, 0x1E, 0x20),
                rail: c(0x23, 0x26, 0x28),
                line: c(0x36, 0x3A, 0x3D),
                text: c(0xE8, 0xE5, 0xDC),
                dim: c(0x9A, 0x97, 0x8C),
                accent: c(0xFF, 0x8A, 0x4C),
                accent_fg: c(0x1A, 0x10, 0x08),
                cell: c(0x2A, 0x2D, 0x30),
                cell_hold: c(0x33, 0x37, 0x3B),
                playhead: c(0xFF, 0x5A, 0x5A),
                metal: c(0x8A, 0x90, 0x96),
                metal_dark: c(0x3C, 0x41, 0x46),
            }
        } else {
            Self {
                dark,
                desk: c(0xD7, 0xD2, 0xC4),
                panel: c(0xF0, 0xEC, 0xE2),
                rail: c(0xE7, 0xE2, 0xD5),
                line: c(0xC9, 0xC2, 0xAF),
                text: c(0x27, 0x25, 0x20),
                dim: c(0x70, 0x6B, 0x5D),
                accent: c(0xD9, 0x48, 0x0F),
                accent_fg: c(0xFF, 0xFF, 0xFF),
                cell: c(0xFB, 0xF9, 0xF2),
                cell_hold: c(0xEC, 0xE6, 0xD4),
                playhead: c(0xD9, 0x30, 0x30),
                metal: c(0xB8, 0xBD, 0xC2),
                metal_dark: c(0x7C, 0x82, 0x88),
            }
        }
    }

    pub fn of(ctx: &Context) -> Self {
        Self::new(ctx.global_style().visuals.dark_mode)
    }

    pub fn accent_soft(&self) -> Color32 {
        self.accent.gamma_multiply(if self.dark { 0.28 } else { 0.18 })
    }
}

fn visuals(dark: bool) -> Visuals {
    let p = Palette::new(dark);
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    let r = CornerRadius::same(6);
    v.override_text_color = Some(p.text);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.line);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = if dark { p.desk } else { p.cell };
    v.faint_bg_color = p.rail;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent_soft();
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.slider_trailing_fill = true;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.line;
    w.inactive.weak_bg_fill = p.rail;
    w.inactive.bg_stroke = Stroke::new(1.0, p.line);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;

    let hover = if dark { p.cell_hold } else { p.cell_hold };
    w.hovered.bg_fill = hover;
    w.hovered.weak_bg_fill = hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.dim);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;

    w.active.bg_fill = p.accent_soft();
    w.active.weak_bg_fill = p.accent_soft();
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = r;

    w.open = w.hovered;
    v
}

/// Installs both themes and the type scale. Egui picks the variant matching the OS.
pub fn install(ctx: &Context) {
    ctx.set_visuals_of(Theme::Light, visuals(false));
    ctx.set_visuals_of(Theme::Dark, visuals(true));
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(6.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.window_margin = Margin::same(14);
        s.spacing.interact_size.y = 26.0;
        s.spacing.slider_width = 120.0;
        s.interaction.tooltip_delay = 0.35;
    });
}
