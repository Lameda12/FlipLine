//! Modal dialogs: new project, export options, unsaved-changes confirmation, help overlay and
//! the export progress card.

use eframe::egui::{self, Align2, Context, Id, Modal, ProgressBar, RichText, Ui};

use super::theme::Palette;
use super::widgets::{self as w};
use crate::app::{FlipLineApp, Pending};
use crate::export::{Format, Scale};
use crate::model::project::SIZE_PRESETS;
use crate::platform;

pub fn show(app: &mut FlipLineApp, ctx: &Context) {
    new_project(app, ctx);
    export(app, ctx);
    confirm(app, ctx);
    help(app, ctx);
    progress(app, ctx);
}

fn confirm(app: &mut FlipLineApp, ctx: &Context) {
    if app.ui.confirm.is_none() {
        return;
    }
    let mut decision = None;
    let modal = Modal::new(Id::new("confirm")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading("Discard unsaved changes?");
        ui.add_space(6.0);
        ui.label("Your current project has changes that haven't been saved to a file. An autosave copy is kept until you start editing the next project.");
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if w::accent_button(ui, "Discard and continue").clicked() {
                decision = Some(true);
            }
            if w::pill(ui, "Cancel", false).clicked() {
                decision = Some(false);
            }
        });
    });
    if modal.should_close() && decision.is_none() {
        decision = Some(false);
    }
    match decision {
        Some(true) => {
            if let Some(p) = app.ui.confirm.take() {
                app.perform(p);
            }
        }
        Some(false) => app.ui.confirm = None,
        None => {}
    }
}

fn new_project(app: &mut FlipLineApp, ctx: &Context) {
    let Some(d) = app.ui.new_dialog.as_mut() else { return };
    let mut create = false;
    let mut sample = false;
    let mut cancel = false;
    let pal = Palette::of(ctx);
    let modal = Modal::new(Id::new("new_project")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading("New project");
        ui.add_space(8.0);
        w::label_caps(ui, "Name");
        ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY));
        ui.add_space(8.0);
        w::label_caps(ui, "Canvas size");
        ui.horizontal_wrapped(|ui| {
            for (i, (label, ..)) in SIZE_PRESETS.iter().enumerate() {
                if w::pill(ui, label, d.size_idx == i).clicked() {
                    d.size_idx = i;
                }
            }
        });
        ui.label(RichText::new("Size is fixed for the whole project.").size(11.5).color(pal.dim));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if w::accent_button(ui, "Create").clicked() {
                create = true;
            }
            if w::pill(ui, "Load sample", false).on_hover_text("The 12-frame bouncing ball").clicked() {
                sample = true;
            }
            if w::pill(ui, "Cancel", false).clicked() {
                cancel = true;
            }
        });
    });
    let (name, idx) = (d.name.trim().to_string(), d.size_idx);
    if create {
        let (_, w_, h_) = SIZE_PRESETS[idx.min(SIZE_PRESETS.len() - 1)];
        app.ui.new_dialog = None;
        let name = if name.is_empty() { "Untitled".to_string() } else { name };
        app.request(Pending::NewProject { name, width: w_, height: h_ });
    } else if sample {
        app.ui.new_dialog = None;
        app.request(Pending::Sample);
    } else if cancel || modal.should_close() {
        app.ui.new_dialog = None;
    }
}

fn export(app: &mut FlipLineApp, ctx: &Context) {
    if app.ui.export_dialog.is_none() {
        return;
    }
    let (pw, ph) = (app.project.width(), app.project.height());
    let frames = app.project.len();
    let ticks = app.project.total_ticks();
    let (fps, secs) = (app.project.fps, app.project.duration_secs());
    let ffmpeg = app.ffmpeg_ok && !platform::is_web();
    let tx = app.sender();
    let pal = Palette::of(ctx);
    let mut go: Option<(Format, String)> = None;
    let mut cancel = false;

    let d = app.ui.export_dialog.as_mut().expect("checked above");
    let modal = Modal::new(Id::new("export")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.heading("Export");
        ui.add_space(8.0);

        w::label_caps(ui, "Format");
        ui.horizontal_wrapped(|ui| {
            let mut formats = vec![Format::Gif, Format::PngSequence, Format::SpriteSheet];
            if ffmpeg {
                formats.push(Format::Mp4);
            }
            for f in formats {
                if w::pill(ui, f.label(), d.opts.format == f).clicked() && d.opts.format != f {
                    d.opts.format = f;
                    d.opts.transparent = matches!(f, Format::PngSequence | Format::SpriteSheet);
                }
            }
        });
        if !ffmpeg {
            let why = if platform::is_web() {
                "MP4 export is available in the desktop app."
            } else {
                "MP4 needs ffmpeg on your PATH (not found)."
            };
            ui.label(RichText::new(why).size(11.5).color(pal.dim));
        }

        ui.add_space(8.0);
        w::label_caps(ui, "Scale");
        ui.horizontal(|ui| {
            for (label, s) in [("100%", Scale::Full), ("50%", Scale::Half)] {
                if w::pill(ui, label, d.opts.scale == s).clicked() {
                    d.opts.scale = s;
                }
            }
            let (ow, oh) = d.opts.scale.dims(pw, ph);
            ui.label(RichText::new(format!("{ow}×{oh} px")).monospace().color(pal.dim));
        });

        ui.add_space(8.0);
        match d.opts.format {
            Format::Gif => {
                ui.checkbox(&mut d.opts.transparent, "Transparent background (1-bit alpha)");
            }
            Format::PngSequence => {
                ui.checkbox(&mut d.opts.transparent, "Transparent background");
                ui.checkbox(&mut d.opts.expand_holds, "One image per exposure (keeps hold timing)");
            }
            Format::SpriteSheet => {
                ui.label(RichText::new("Transparent sheet, one cell per drawing, near-square grid.").color(pal.dim));
            }
            Format::Mp4 => {
                ui.label(RichText::new("H.264, drawn over the paper colour.").color(pal.dim));
            }
        }

        ui.add_space(6.0);
        let what = match d.opts.format {
            Format::Gif => "Per-frame 256-colour palette, loops forever.",
            Format::PngSequence => "Numbered PNGs in a folder (a .zip download on the web).",
            Format::SpriteSheet => "A single PNG.",
            Format::Mp4 => "Timing follows holds exactly at the project fps.",
        };
        ui.label(RichText::new(format!("{frames} frames · {ticks} exposures · {fps} fps · {secs:.2}s. {what}")).size(11.5).color(pal.dim));

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if w::accent_button(ui, if d.choosing { "Choosing…" } else { "Export…" }).clicked() && !d.choosing {
                go = Some((d.opts.format, crate::export::sanitize_stem(&d.opts.base_name)));
            }
            if w::pill(ui, "Cancel", false).clicked() {
                cancel = true;
            }
        });
    });
    if let Some((format, stem)) = go {
        d.choosing = true;
        platform::choose_export_dest(format, stem, tx, ctx.clone());
    }
    if cancel || (modal.should_close() && !app.ui.export_dialog.as_ref().is_some_and(|d| d.choosing)) {
        app.ui.export_dialog = None;
    }
}

fn progress(app: &mut FlipLineApp, ctx: &Context) {
    let Some((done, total, label)) = app.export_progress() else { return };
    let pal = Palette::of(ctx);
    let mut cancel = false;
    egui::Area::new(Id::new("export_progress"))
        .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -170.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(pal.panel)
                .stroke(egui::Stroke::new(1.0, pal.line))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(14))
                .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: egui::Color32::from_black_alpha(70) })
                .show(ui, |ui| {
                    ui.set_width(320.0);
                    ui.label(RichText::new(format!("Exporting {label}")).strong());
                    ui.add_space(4.0);
                    let frac = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                    ui.add(ProgressBar::new(frac).text(format!("{done}/{total}")).animate(true));
                    ui.add_space(4.0);
                    if w::pill(ui, "Cancel", false).clicked() {
                        cancel = true;
                    }
                });
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
    if cancel {
        app.cancel_export();
    }
}

fn help(app: &mut FlipLineApp, ctx: &Context) {
    if !app.ui.help {
        return;
    }
    let pal = Palette::of(ctx);
    let cmd = if ctx.os() == egui::os::OperatingSystem::Mac { "Cmd" } else { "Ctrl" };
    let sections: Vec<(&str, Vec<(String, &str)>)> = vec![
        (
            "Tools",
            vec![
                ("B".into(), "Brush"),
                ("E".into(), "Eraser"),
                ("G".into(), "Fill"),
                ("I".into(), "Colour picker"),
                ("[  ]".into(), "Smaller / larger"),
            ],
        ),
        (
            "Frames",
            vec![
                (", .  ← →".into(), "Previous / next"),
                ("N".into(), "New blank frame"),
                ("D".into(), "Duplicate frame"),
                ("Delete".into(), "Delete frame"),
                ("Alt + ← →".into(), "Move frame"),
                ("-  =".into(), "Shorter / longer hold"),
            ],
        ),
        (
            "Playback & view",
            vec![
                ("Space".into(), "Play / pause"),
                ("O".into(), "Onion skin"),
                ("Space + drag".into(), "Pan (or middle mouse)"),
                (format!("{cmd} + wheel"), "Zoom (or pinch)"),
                ("F  /  1".into(), "Fit  /  100%"),
            ],
        ),
        (
            "File",
            vec![
                (format!("{cmd} + Z"), "Undo"),
                (format!("Shift + {cmd} + Z"), "Redo"),
                (format!("{cmd} + S"), "Save"),
                (format!("Shift + {cmd} + S"), "Save as"),
                (format!("{cmd} + E"), "Export"),
                ("?".into(), "This help"),
            ],
        ),
    ];
    let modal = Modal::new(Id::new("help")).show(ctx, |ui| {
        ui.set_width(560.0);
        ui.heading("Keyboard shortcuts");
        ui.add_space(8.0);
        ui.columns(2, |cols| {
            for (i, (title, rows)) in sections.iter().enumerate() {
                let ui: &mut Ui = &mut cols[i % 2];
                w::label_caps(ui, title);
                egui::Grid::new(format!("help_{i}")).num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
                    for (k, what) in rows {
                        ui.label(RichText::new(k).monospace().color(pal.accent));
                        ui.label(*what);
                        ui.end_row();
                    }
                });
                ui.add_space(10.0);
            }
        });
        ui.label(RichText::new("Drop a numbered image sequence on the window to import it as frames.").size(11.5).color(pal.dim));
    });
    if modal.should_close() {
        app.ui.help = false;
    }
}
