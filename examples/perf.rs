//! Micro-benchmarks for the hot paths of a 1280x720, 100-frame project.
//! Run with: cargo run --release --example perf
//! A 60 fps frame budget is 16.7 ms; everything per-frame below should be a small fraction of it.

use std::time::Instant;

use eframe::egui;
use flipline::app::FlipLineApp;
use flipline::model::Project;
use flipline::render;
use flipline::tools::{Sample, ToolId};

fn time<R>(label: &str, iters: u32, mut f: impl FnMut() -> R) -> f64 {
    for _ in 0..2 {
        std::hint::black_box(f());
    }
    let t = Instant::now();
    for _ in 0..iters {
        std::hint::black_box(f());
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    println!("{label:<58} {ms:>9.3} ms");
    ms
}

fn scribble(app: &mut FlipLineApp, seed: f32, events: usize) {
    app.pointer_down(Sample::new(100.0 + seed, 300.0, 1.0));
    for i in 1..=events {
        let t = i as f32 / events as f32;
        app.pointer_drag(Sample::new(
            100.0 + seed + t * 1000.0,
            300.0 + (t * 24.0 + seed).sin() * 200.0,
            0.3 + 0.7 * t,
        ));
    }
    app.pointer_up();
}

fn main() {
    let mut p = Project::new("perf", 1280, 720);
    for _ in 1..100 {
        let f = p.new_blank_frame();
        let at = p.len();
        p.insert_frame(at, f);
    }
    let mut app = FlipLineApp::with_project(p);
    app.settings.size = 24.0;
    // Fill every frame with realistic line work.
    for i in 0..100 {
        app.set_current(i);
        for s in 0..6 {
            scribble(&mut app, (i * 7 + s * 40) as f32, 120);
        }
    }
    app.set_current(50);
    let resident: usize = app
        .project
        .frames()
        .iter()
        .filter_map(|f| f.pixmap())
        .map(|p| p.data().len())
        .sum();
    println!("100 drawn frames resident: {:.0} MB\n", resident as f64 / 1e6);

    println!("-- drawing (per pointer event / per gesture) --");
    // One pointer event = smoothing + capsule rasterisation of the new samples.
    app.pointer_down(Sample::new(100.0, 300.0, 1.0));
    let mut i = 0.0f32;
    let per_event = time(
        "brush: one pointer event (24 px brush, ~16 px move)",
        2000,
        || {
            i += 1.0;
            app.pointer_drag(Sample::new(
                100.0 + (i * 16.0) % 1000.0,
                300.0 + (i * 0.3).sin() * 200.0,
                0.8,
            ));
        },
    );
    app.pointer_up();
    println!(
        "    => a 1000 Hz mouse delivers ~16 events per 60 fps frame = {:.2} ms/frame",
        per_event * 16.0
    );
    time("brush: gesture start (copy-on-write of the frame)", 200, || {
        app.pointer_down(Sample::new(300.0, 300.0, 1.0));
        app.cancel_stroke();
    });
    time(
        "brush: gesture end (before/after dirty rect for undo)",
        100,
        || {
            app.pointer_down(Sample::new(100.0, 300.0, 1.0));
            app.pointer_drag(Sample::new(900.0, 500.0, 1.0));
            app.pointer_up();
            app.undo();
        },
    );
    app.set_tool(ToolId::Fill);
    time("fill: click on a mostly-empty frame (large region)", 50, || {
        app.pointer_down(Sample::new(5.0, 5.0, 1.0));
        app.undo();
    });
    app.set_tool(ToolId::Brush);

    println!("\n-- caches --");
    let ctx = egui::Context::default();
    let drawn = app.project.frame(50).unwrap().pixmap().unwrap().clone();
    let neighbours: Vec<_> = [-2i32, -1, 1, 2]
        .iter()
        .map(|d| {
            (
                app.project
                    .frame((50 + d) as usize)
                    .unwrap()
                    .pixmap()
                    .unwrap()
                    .clone(),
                *d,
            )
        })
        .collect();
    time("onion: rebuild composite (depth 2 = 4 layers)", 20, || {
        let layers: Vec<_> = neighbours
            .iter()
            .map(|(pm, d)| render::OnionLayer {
                distance: *d,
                pixmap: Some(pm),
            })
            .collect();
        render::compose_onion(1280, 720, &layers)
    });
    time(
        "thumbnail: one frame (box filter 1280x720 -> 128x72)",
        100,
        || render::make_thumbnail(Some(&drawn), 1280, 720, [250, 247, 238]),
    );
    time("texture: full upload prep (ColorImage from 1280x720)", 50, || {
        egui::ColorImage::from_rgba_premultiplied([1280, 720], drawn.data())
    });
    time(
        "texture: partial upload prep (typical 200x150 stroke rect)",
        2000,
        || {
            let r = flipline::model::IRect::new(300, 200, 500, 350);
            let b = flipline::model::pixels::extract_rect(&drawn, r);
            egui::ColorImage::from_rgba_premultiplied([200, 150], &b)
        },
    );

    println!("\n-- whole UI pass (layout + tessellation, no GPU) --");
    let mut t = 0.0;
    let mut frame_ms = 0.0;
    for label in ["100 frames, idle", "100 frames, onion on + dialog closed"] {
        // Warm up fonts/textures first.
        for _ in 0..5 {
            t += 0.016;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    time: Some(t),
                    ..Default::default()
                },
                |ui| eframe::App::ui(&mut app, ui, &mut unsafe_frame()),
            );
            ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
            out.textures_delta.clear();
        }
        frame_ms = time(&format!("egui pass + tessellate: {label}"), 60, || {
            t += 0.016;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    time: Some(t),
                    ..Default::default()
                },
                |ui| eframe::App::ui(&mut app, ui, &mut unsafe_frame()),
            );
            let tris = ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
            out.textures_delta.clear();
            tris.len()
        });
        app.onion.enabled = !app.onion.enabled;
    }
    println!("\nbudget: 16.7 ms/frame at 60 fps. UI pass above is CPU only; GPU upload/draw not included.");
    let _ = frame_ms;
}

/// `eframe::Frame` cannot be built outside eframe; the app never reads it, so the benchmark uses
/// eframe's headless test constructor.
fn unsafe_frame() -> eframe::Frame {
    eframe::Frame::_new_kittest()
}
