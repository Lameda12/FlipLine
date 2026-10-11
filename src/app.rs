//! Application state and editing operations. UI modules (`ui/`) stay thin: they read this state,
//! call these methods, and paint.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Context, Vec2};
use serde::{Deserialize, Serialize};

use crate::autosave::Autosave;
use crate::export::{Dest, ExportJob, ExportOptions, ExportOutput, Format, Step};
use crate::history::{Command, Composite, FrameSpan, History, MoveFrame, SetHold};
use crate::model::io::{decode_image_fitted, from_bytes};
use crate::model::timing::{self, MAX_HOLD, MIN_HOLD};
use crate::model::Project;
use crate::platform::{self, Msg, PickedFile};
use crate::render::{self, OnionCache};
use crate::sample;
use crate::tools::{Sample, ToolId, ToolOutput, ToolSettings, Toolbox, MAX_SIZE, MIN_SIZE};
use crate::ui::toasts::Toasts;

pub const SWATCHES: [[u8; 3]; 8] = [
    [0x1B, 0x1B, 0x22],
    [0x6B, 0x70, 0x7A],
    [0xD7, 0x26, 0x3D],
    [0xF4, 0x7B, 0x20],
    [0xF2, 0xB7, 0x05],
    [0x2E, 0x93, 0x3C],
    [0x25, 0x63, 0xEB],
    [0x7B, 0x2C, 0xBF],
];

pub enum ZoomRequest {
    Fit,
    Actual,
    By(f32),
}

pub struct View {
    pub request: Option<ZoomRequest>,
    /// Physical screen pixels per canvas pixel.
    pub zoom: f32,
    /// Offset of the canvas centre from the viewport centre, in points.
    pub pan: Vec2,
    /// While true the canvas re-fits whenever the window changes.
    pub fit: bool,
}

impl Default for View {
    fn default() -> Self {
        Self { request: None, zoom: 1.0, pan: Vec2::ZERO, fit: true }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct OnionSettings {
    pub enabled: bool,
    pub depth: u8,
}

impl Default for OnionSettings {
    fn default() -> Self {
        Self { enabled: true, depth: 2 }
    }
}

pub struct Playback {
    pub playing: bool,
    pub looping: bool,
    start_time: f64,
    offset_secs: f64,
}

pub struct StrokeState {
    pub tool: ToolId,
    pub last: Sample,
}

#[derive(Default)]
pub struct Input {
    pub space_down: bool,
    pub space_used: bool,
    /// Latest pen/touch force, if the platform provides one.
    pub pressure: Option<f32>,
    pub panning: bool,
    pub saw_pressure: bool,
}

pub enum Pending {
    NewProject { name: String, width: u32, height: u32 },
    OpenBytes { name: String, bytes: Vec<u8>, path: Option<PathBuf> },
    Sample,
}

pub struct NewProjectDialog {
    pub name: String,
    pub size_idx: usize,
}

pub struct ExportDialog {
    pub opts: ExportOptions,
    /// A destination picker is open.
    pub choosing: bool,
}

pub struct ExportRun {
    pub total: usize,
    pub progress: Arc<AtomicUsize>,
    pub cancel: Arc<AtomicBool>,
    /// Set on the web, where the job is stepped from the UI thread.
    inline: Option<ExportJob>,
    pub label: &'static str,
    pub stem: String,
}

#[derive(Default)]
pub struct UiState {
    pub help: bool,
    pub new_dialog: Option<NewProjectDialog>,
    pub export_dialog: Option<ExportDialog>,
    pub confirm: Option<Pending>,
    pub drop_hover: bool,
    /// Timeline cell being dragged for reordering.
    pub drag_from: Option<usize>,
}

#[derive(Serialize, Deserialize)]
struct Prefs {
    color: [u8; 3],
    size: f32,
    tolerance: u8,
    tool: String,
    onion: OnionSettings,
    looping: bool,
}

pub struct FlipLineApp {
    pub project: Project,
    pub history: History,
    pub current: usize,
    pub file_path: Option<PathBuf>,
    clean_gen: u64,
    local_gen: u64,
    pending_save_gen: Option<u64>,

    pub tools: Toolbox,
    pub tool: ToolId,
    pub return_tool: ToolId,
    pub settings: ToolSettings,
    pub stroke: Option<StrokeState>,

    pub view: View,
    pub onion: OnionSettings,
    pub onion_cache: OnionCache,
    pub playback: Playback,
    pub input: Input,
    pub scroll_to_current: bool,

    pub ui: UiState,
    pub toasts: Toasts,
    pub export_run: Option<ExportRun>,
    pub ffmpeg_ok: bool,
    pub autosave: Autosave,

    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    title: String,
}

fn tool_name(t: ToolId) -> &'static str {
    t.name()
}

fn tool_from_name(n: &str) -> ToolId {
    ToolId::ALL.into_iter().find(|t| tool_name(*t) == n).unwrap_or(ToolId::Brush)
}

impl FlipLineApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::ui::theme::install(&cc.egui_ctx);
        let (tx, rx) = channel();
        platform::check_ffmpeg(tx.clone(), cc.egui_ctx.clone());

        let mut toasts = Toasts::default();
        let mut restored = false;
        let project = match platform::autosave_read().map(|b| from_bytes(&b)) {
            Some(Ok(p)) => {
                restored = true;
                p
            }
            Some(Err(e)) => {
                toasts.error(format!("Couldn't restore the autosave ({e}); starting with the sample"));
                sample::bouncing_ball()
            }
            None => sample::bouncing_ball(),
        };

        let mut app = Self {
            project,
            history: History::new(),
            current: 0,
            file_path: None,
            clean_gen: 0,
            local_gen: 0,
            pending_save_gen: None,
            tools: Toolbox::default(),
            tool: ToolId::Brush,
            return_tool: ToolId::Brush,
            settings: ToolSettings::default(),
            stroke: None,
            view: View::default(),
            onion: OnionSettings::default(),
            onion_cache: OnionCache::default(),
            playback: Playback { playing: false, looping: true, start_time: 0.0, offset_secs: 0.0 },
            input: Input::default(),
            scroll_to_current: true,
            ui: UiState::default(),
            toasts,
            export_run: None,
            ffmpeg_ok: false,
            autosave: Autosave::default(),
            tx,
            rx,
            title: String::new(),
        };
        if let Some(storage) = cc.storage {
            if let Some(p) = storage.get_string("prefs").and_then(|s| serde_json::from_str::<Prefs>(&s).ok()) {
                app.settings.color = p.color;
                app.settings.size = p.size.clamp(MIN_SIZE, MAX_SIZE);
                app.settings.tolerance = p.tolerance;
                app.tool = tool_from_name(&p.tool);
                if matches!(app.tool, ToolId::Picker) {
                    app.tool = ToolId::Brush;
                }
                app.return_tool = app.tool;
                app.onion = OnionSettings { enabled: p.onion.enabled, depth: p.onion.depth.clamp(1, 3) };
                app.playback.looping = p.looping;
            }
        }
        // A restored autosave has never been saved to a file, so it counts as modified.
        app.clean_gen = if restored { u64::MAX } else { app.generation() };
        app.autosave.mark_clean(app.generation());
        if restored {
            app.toasts.info("Restored your autosaved project");
        }
        app
    }

    // ---- bookkeeping -----------------------------------------------------------------------

    /// Monotonic counter that changes on every edit (history, settings, renames).
    pub fn generation(&self) -> u64 {
        self.history.generation() + self.local_gen
    }

    pub fn is_modified(&self) -> bool {
        self.generation() != self.clean_gen
    }

    pub fn mark_modified(&mut self) {
        self.local_gen += 1;
    }

    pub fn sender(&self) -> Sender<Msg> {
        self.tx.clone()
    }

    pub fn frame_count(&self) -> usize {
        self.project.len()
    }

    // ---- selection & playback --------------------------------------------------------------

    pub fn set_current(&mut self, idx: usize) {
        self.finish_stroke();
        let idx = idx.min(self.project.len().saturating_sub(1));
        if idx != self.current {
            self.current = idx;
            self.scroll_to_current = true;
        }
    }

    pub fn step_frame(&mut self, delta: i32) {
        self.stop_playback();
        let idx = (self.current as i32 + delta).clamp(0, self.project.len() as i32 - 1);
        self.set_current(idx as usize);
    }

    pub fn toggle_playback(&mut self, now: f64) {
        if self.playback.playing {
            self.stop_playback();
        } else {
            self.finish_stroke();
            self.playback.playing = true;
            self.playback.start_time = now;
            let holds = self.project.holds();
            // Epsilon keeps float rounding from landing one tick early.
            self.playback.offset_secs =
                timing::ticks_to_secs(timing::start_tick(&holds, self.current), self.project.fps) + 1e-6;
        }
    }

    pub fn stop_playback(&mut self) {
        self.playback.playing = false;
    }

    /// Restarts the playback clock at the frame on screen (after fps changes while playing).
    pub fn rebase_playback(&mut self, now: f64) {
        if self.playback.playing {
            self.playback.playing = false;
            self.toggle_playback(now);
        }
    }

    pub fn tick_playback(&mut self, ctx: &Context) {
        if !self.playback.playing {
            return;
        }
        let now = ctx.input(|i| i.time);
        let elapsed = now - self.playback.start_time + self.playback.offset_secs;
        let holds = self.project.holds();
        let ph = timing::playhead(&holds, elapsed, self.project.fps, self.playback.looping);
        if ph.frame != self.current {
            self.current = ph.frame;
            self.scroll_to_current = true;
        }
        if ph.finished && !self.playback.looping {
            self.playback.playing = false;
        } else {
            let wait = timing::secs_until_next_tick(elapsed, self.project.fps).max(0.001);
            ctx.request_repaint_after(Duration::from_secs_f64(wait));
        }
    }

    pub fn set_fps(&mut self, fps: u32, now: f64) {
        if fps != self.project.fps {
            self.project.fps = fps;
            self.mark_modified();
            self.rebase_playback(now);
        }
    }

    // ---- tools -----------------------------------------------------------------------------

    pub fn set_tool(&mut self, t: ToolId) {
        self.finish_stroke();
        if t != ToolId::Picker {
            self.return_tool = t;
        }
        self.tool = t;
    }

    pub fn adjust_size(&mut self, up: bool) {
        let s = self.settings.size;
        let step = (s * 0.18).round().max(1.0);
        self.settings.size = (if up { s + step } else { s - step }).clamp(MIN_SIZE, MAX_SIZE).round();
    }

    pub fn pointer_down(&mut self, s: Sample) {
        if self.playback.playing {
            return;
        }
        let (tool, settings) = (self.tool, self.settings);
        let Some(frame) = self.project.frame_mut(self.current) else { return };
        let out = self.tools.get(tool).begin(frame, &settings, s);
        let active = self.tools.get(tool).is_active();
        self.handle_tool_output(out);
        if active {
            self.stroke = Some(StrokeState { tool, last: s });
        }
    }

    pub fn pointer_drag(&mut self, s: Sample) {
        let Some(st) = self.stroke.as_mut() else { return };
        st.last = s;
        let (tool, settings) = (st.tool, self.settings);
        if let Some(frame) = self.project.frame_mut(self.current) {
            self.tools.get(tool).drag(frame, &settings, s);
        }
    }

    pub fn pointer_up(&mut self) {
        self.finish_stroke();
    }

    /// Ends any gesture in progress, committing it to history.
    pub fn finish_stroke(&mut self) {
        let Some(st) = self.stroke.take() else { return };
        let settings = self.settings;
        let Some(frame) = self.project.frame_mut(self.current) else { return };
        let out = self.tools.get(st.tool).end(frame, &settings, st.last);
        self.handle_tool_output(out);
    }

    pub fn cancel_stroke(&mut self) {
        if let Some(st) = self.stroke.take() {
            if let Some(frame) = self.project.frame_mut(self.current) {
                self.tools.get(st.tool).cancel(frame);
            }
        }
    }

    fn handle_tool_output(&mut self, out: ToolOutput) {
        match out {
            ToolOutput::None => {}
            ToolOutput::Commit(cmd) => self.history.push_applied(cmd),
            ToolOutput::PickedColor(c) => {
                self.settings.color = c;
                if self.tool == ToolId::Picker {
                    self.tool = self.return_tool;
                }
            }
        }
    }

    // ---- timeline edits (all undoable) ------------------------------------------------------

    fn run(&mut self, cmd: Box<dyn Command>) {
        self.finish_stroke();
        self.stop_playback();
        if let Some(sel) = self.history.execute(cmd, &mut self.project) {
            self.set_current(sel);
        }
    }

    pub fn new_frame(&mut self) {
        if self.project.len() >= crate::model::project::MAX_FRAMES {
            self.toasts.error("Frame limit reached");
            return;
        }
        let f = self.project.new_blank_frame();
        let at = self.current + 1;
        self.run(Box::new(FrameSpan::insert(at, vec![f], "New frame")));
    }

    pub fn duplicate_frame(&mut self) {
        if self.project.len() >= crate::model::project::MAX_FRAMES {
            self.toasts.error("Frame limit reached");
            return;
        }
        if let Some(f) = self.project.duplicate_frame(self.current) {
            let at = self.current + 1;
            self.run(Box::new(FrameSpan::insert(at, vec![f], "Duplicate frame")));
        }
    }

    pub fn delete_frame(&mut self) {
        if self.project.len() <= 1 {
            // Never leave the timeline empty: replace the last drawing with a blank one.
            if self.project.frame(0).is_some_and(|f| f.is_blank()) {
                return;
            }
            let blank = self.project.new_blank_frame();
            let parts: Vec<Box<dyn Command>> = vec![
                Box::new(FrameSpan::insert(1, vec![blank], "Clear frame")),
                Box::new(FrameSpan::remove(0, 1)),
            ];
            self.run(Box::new(Composite { parts, label: "Clear frame" }));
        } else {
            self.run(Box::new(FrameSpan::remove(self.current, 1)));
        }
    }

    pub fn move_frame_by(&mut self, delta: i32) {
        let to = self.current as i32 + delta;
        if to >= 0 && (to as usize) < self.project.len() {
            self.run(Box::new(MoveFrame { from: self.current, to: to as usize }));
        }
    }

    pub fn move_frame_to(&mut self, from: usize, to: usize) {
        if from != to && from < self.project.len() && to < self.project.len() {
            self.run(Box::new(MoveFrame { from, to }));
        }
    }

    pub fn change_hold(&mut self, delta: i32) {
        let Some(f) = self.project.frame(self.current) else { return };
        let new = (f.hold as i32 + delta).clamp(MIN_HOLD as i32, MAX_HOLD as i32) as u8;
        if new != f.hold {
            let cmd = SetHold { frame: f.id, old: f.hold, new };
            self.run(Box::new(cmd));
        }
    }

    pub fn set_hold(&mut self, hold: u8) {
        let cur = self.project.frame(self.current).map(|f| f.hold).unwrap_or(1);
        self.change_hold(hold as i32 - cur as i32);
    }

    pub fn undo(&mut self) {
        self.finish_stroke();
        self.stop_playback();
        if let Some(sel) = self.history.undo(&mut self.project) {
            self.set_current(sel);
        }
    }

    pub fn redo(&mut self) {
        self.finish_stroke();
        self.stop_playback();
        if let Some(sel) = self.history.redo(&mut self.project) {
            self.set_current(sel);
        }
    }

    // ---- project lifecycle ------------------------------------------------------------------

    fn replace_project(&mut self, project: Project, path: Option<PathBuf>, clean: bool) {
        self.finish_stroke();
        self.stop_playback();
        self.project = project;
        self.history.clear();
        self.current = 0;
        self.file_path = path;
        self.view = View::default();
        self.onion_cache.invalidate();
        self.scroll_to_current = true;
        if clean {
            self.clean_gen = self.generation();
        } else {
            self.clean_gen = u64::MAX;
        }
    }

    pub fn request(&mut self, action: Pending) {
        if self.is_modified() {
            self.ui.confirm = Some(action);
        } else {
            self.perform(action);
        }
    }

    pub fn perform(&mut self, action: Pending) {
        match action {
            Pending::NewProject { name, width, height } => {
                let mut p = Project::new(name, width, height);
                p.fps = self.project.fps;
                self.replace_project(p, None, false);
            }
            Pending::OpenBytes { name, bytes, path } => match from_bytes(&bytes) {
                Ok(p) => {
                    self.replace_project(p, path, true);
                    self.toasts.info(format!("Opened {name}"));
                }
                Err(e) => self.toasts.error(format!("Couldn't open {name}: {e}")),
            },
            Pending::Sample => {
                self.replace_project(sample::bouncing_ball(), None, false);
            }
        }
    }

    pub fn save(&mut self, ctx: &Context, save_as: bool) {
        self.finish_stroke();
        let path = if save_as { None } else { self.file_path.clone() };
        self.pending_save_gen = Some(self.generation());
        platform::save_project(self.project.snapshot(), path, self.tx.clone(), ctx.clone(), self.autosave.cache.clone());
    }

    // ---- import ----------------------------------------------------------------------------

    pub fn import_files(&mut self, mut files: Vec<PickedFile>, ctx: &Context) {
        files.sort_by(|a, b| crate::util::natural_cmp(&a.name, &b.name));
        let (w, h) = (self.project.width(), self.project.height());
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        platform::run_background(move || {
            let mut frames = Vec::new();
            let mut errors = Vec::new();
            for f in files {
                match decode_image_fitted(&f.bytes, w, h) {
                    Ok(pm) => frames.push(pm),
                    Err(e) => errors.push(format!("{}: {e}", f.name)),
                }
            }
            let _ = tx.send(Msg::ImagesDecoded { frames, errors });
            ctx.request_repaint();
        });
    }

    fn insert_images(&mut self, pixmaps: Vec<tiny_skia::Pixmap>) {
        if pixmaps.is_empty() {
            return;
        }
        let room = crate::model::project::MAX_FRAMES.saturating_sub(self.project.len());
        let n = pixmaps.len().min(room);
        if n < pixmaps.len() {
            self.toasts.error(format!("Frame limit reached: imported {n} of {}", pixmaps.len()));
        }
        let frames: Vec<_> = pixmaps.into_iter().take(n).map(|pm| self.project.frame_from_pixmap(pm)).collect();
        let replace_blank = self.project.len() == 1 && self.project.frame(0).is_some_and(|f| f.is_blank());
        let count = frames.len();
        if replace_blank {
            let parts: Vec<Box<dyn Command>> = vec![
                Box::new(FrameSpan::insert(1, frames, "Import images")),
                Box::new(FrameSpan::remove(0, 1)),
            ];
            self.run(Box::new(Composite { parts, label: "Import images" }));
            self.set_current(0);
        } else {
            let at = self.current + 1;
            self.run(Box::new(FrameSpan::insert(at, frames, "Import images")));
        }
        self.toasts.info(format!("Imported {count} image{}", if count == 1 { "" } else { "s" }));
    }

    // ---- export ----------------------------------------------------------------------------

    pub fn open_export_dialog(&mut self) {
        self.finish_stroke();
        let mut opts = ExportOptions::new(Format::Gif);
        opts.base_name = self.project.name.clone();
        self.ui.export_dialog = Some(ExportDialog { opts, choosing: false });
    }

    pub fn start_export(&mut self, dest: Dest, opts: ExportOptions, ctx: &Context) {
        let snap = self.project.snapshot();
        let job = match ExportJob::new(snap, opts.clone(), dest) {
            Ok(j) => j,
            Err(e) => {
                self.toasts.error(format!("Export failed: {e}"));
                return;
            }
        };
        let run_meta = (job.total_steps(), job.progress_counter(), job.cancel_flag());
        let stem = crate::export::sanitize_stem(&opts.base_name);
        let label = opts.format.label();
        if platform::is_web() {
            self.export_run = Some(ExportRun {
                total: run_meta.0,
                progress: run_meta.1,
                cancel: run_meta.2,
                inline: Some(job),
                label,
                stem,
            });
        } else {
            self.export_run = Some(ExportRun {
                total: run_meta.0,
                progress: run_meta.1,
                cancel: run_meta.2,
                inline: None,
                label,
                stem,
            });
            let (tx, ctx) = (self.tx.clone(), ctx.clone());
            platform::run_background(move || {
                let result = job.run();
                let _ = tx.send(Msg::ExportFinished(result));
                ctx.request_repaint();
            });
        }
    }

    fn finish_export(&mut self, result: Result<ExportOutput, crate::export::ExportError>) {
        let run = self.export_run.take();
        match result {
            Ok(out) => {
                if !out.files.is_empty() {
                    let stem = run.as_ref().map(|r| r.stem.clone()).unwrap_or_else(|| "flipline".into());
                    if let Err(e) = platform::deliver_memory_output(out.files, &stem) {
                        self.toasts.error(format!("Export failed: {e}"));
                        return;
                    }
                }
                self.toasts.info(out.summary);
            }
            Err(crate::export::ExportError::Cancelled) => self.toasts.info("Export cancelled"),
            Err(e) => self.toasts.error(format!("Export failed: {e}")),
        }
    }

    /// Time-slices a web export on the UI thread.
    fn step_inline_export(&mut self, ctx: &Context) {
        let Some(run) = self.export_run.as_mut() else { return };
        let Some(job) = run.inline.as_mut() else { return };
        let start = ctx.input(|i| i.time);
        let mut finished = None;
        loop {
            match job.step() {
                Ok(Step::More) => {}
                Ok(Step::Done(out)) => {
                    finished = Some(Ok(out));
                    break;
                }
                Err(e) => {
                    finished = Some(Err(e));
                    break;
                }
            }
            if ctx.input(|i| i.time) - start > 0.012 || start == 0.0 {
                break;
            }
        }
        match finished {
            Some(r) => self.finish_export(r),
            None => ctx.request_repaint(),
        }
    }

    // ---- per-frame housekeeping ------------------------------------------------------------

    pub fn begin_frame(&mut self, ctx: &Context) {
        self.poll_messages(ctx);
        self.handle_drops(ctx);
        self.step_inline_export(ctx);
        self.tick_playback(ctx);

        if !self.playback.playing {
            render::evict_far_textures(&mut self.project, self.current, 3);
        }

        let now = ctx.input(|i| i.time);
        let busy = self.stroke.is_some() || self.playback.playing;
        let generation = self.generation();
        let project = &self.project;
        self.autosave.maybe_run(now, generation, busy, || project.snapshot(), &self.tx, ctx);

        let title = format!(
            "{}{} · FlipLine",
            self.project.name,
            if self.is_modified() { " •" } else { "" }
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn poll_messages(&mut self, ctx: &Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::ProjectOpened { name, bytes, path } => {
                    self.request(Pending::OpenBytes { name, bytes, path });
                }
                Msg::ImagesPicked(files) => self.import_files(files, ctx),
                Msg::ImagesDecoded { frames, errors } => {
                    for e in errors.iter().take(3) {
                        self.toasts.error(format!("Couldn't import {e}"));
                    }
                    self.insert_images(frames);
                }
                Msg::Saved { path, name } => {
                    if let Some(g) = self.pending_save_gen.take() {
                        self.clean_gen = g;
                    }
                    if path.is_some() {
                        self.file_path = path;
                    }
                    self.toasts.info(format!("Saved {name}"));
                }
                Msg::ExportDest(dest) => {
                    if let Some(d) = self.ui.export_dialog.take() {
                        self.start_export(dest, d.opts, ctx);
                    }
                }
                Msg::ExportDestCancelled => {
                    if let Some(d) = self.ui.export_dialog.as_mut() {
                        d.choosing = false;
                    }
                }
                Msg::ExportFinished(r) => self.finish_export(r),
                Msg::AutosaveDone(r) => {
                    self.autosave.finished();
                    if let Err(e) = r {
                        self.toasts.error(format!("Autosave failed: {e}"));
                    }
                }
                Msg::FfmpegChecked(ok) => self.ffmpeg_ok = ok,
                Msg::Info(s) => self.toasts.info(s),
                Msg::Error(s) => self.toasts.error(s),
            }
        }
    }

    fn handle_drops(&mut self, ctx: &Context) {
        let (hovering, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.clone()));
        self.ui.drop_hover = hovering;
        if dropped.is_empty() {
            return;
        }
        platform::read_dropped(dropped, self.tx.clone(), ctx.clone());
    }

    pub fn export_progress(&self) -> Option<(usize, usize, &'static str)> {
        self.export_run
            .as_ref()
            .map(|r| (r.progress.load(Ordering::Relaxed), r.total, r.label))
    }

    pub fn cancel_export(&mut self) {
        if let Some(r) = &self.export_run {
            r.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl eframe::App for FlipLineApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.begin_frame(&ctx);
        crate::ui::shortcuts::handle(self, &ctx);
        crate::ui::topbar::show(self, ui);
        crate::ui::toolbar::show(self, ui);
        crate::ui::timeline::show(self, ui);
        crate::ui::canvas::show(self, ui);
        crate::ui::dialogs::show(self, &ctx);
        self.toasts.show(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let prefs = Prefs {
            color: self.settings.color,
            size: self.settings.size,
            tolerance: self.settings.tolerance,
            tool: tool_name(self.return_tool).to_string(),
            onion: self.onion,
            looping: self.playback.looping,
        };
        if let Ok(s) = serde_json::to_string(&prefs) {
            storage.set_string("prefs", s);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}
