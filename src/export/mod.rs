//! Export pipeline. Every format is a step-based job (one frame per step) so the same code runs
//! on a background thread natively and is time-sliced on the UI thread on wasm, with a progress
//! counter either way.

mod ffmpeg;
mod gif_out;
mod png_out;
mod sheet;

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;
use tiny_skia::Pixmap;

use crate::model::pixels::{downscale_half, flatten_over, pixmap_to_straight};
use crate::model::project::FrameSnapshot;
use crate::model::Snapshot;

pub use ffmpeg::{ffmpeg_available, ffmpeg_command};

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("image encoding failed: {0}")]
    Image(#[from] image::ImageError),
    #[error("GIF encoding failed: {0}")]
    Gif(#[from] gif::EncodingError),
    #[error("ffmpeg: {0}")]
    Ffmpeg(String),
    #[error("sprite sheet would be {w}×{h} px ({mb} MB); try 50% scale")]
    SheetTooLarge { w: u64, h: u64, mb: u64 },
    #[error("nothing to export")]
    Empty,
    #[error("export cancelled")]
    Cancelled,
    #[error("{0}")]
    Unsupported(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Gif,
    PngSequence,
    SpriteSheet,
    Mp4,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Gif => "Animated GIF",
            Format::PngSequence => "PNG sequence",
            Format::SpriteSheet => "Sprite sheet",
            Format::Mp4 => "MP4 video",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Gif => "gif",
            Format::PngSequence => "png",
            Format::SpriteSheet => "png",
            Format::Mp4 => "mp4",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Full,
    Half,
}

impl Scale {
    pub fn dims(self, w: u32, h: u32) -> (u32, u32) {
        match self {
            Scale::Full => (w, h),
            Scale::Half => ((w / 2).max(1), (h / 2).max(1)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub format: Format,
    pub scale: Scale,
    /// Keep transparency (PNG, sprite sheet; GIF with 1-bit alpha). MP4 is always opaque.
    pub transparent: bool,
    /// PNG sequence: write one image per exposure so playback at the project fps keeps the timing.
    pub expand_holds: bool,
    /// File stem used for generated names.
    pub base_name: String,
}

impl ExportOptions {
    pub fn new(format: Format) -> Self {
        Self {
            format,
            scale: Scale::Full,
            transparent: matches!(format, Format::PngSequence | Format::SpriteSheet),
            expand_holds: true,
            base_name: "flipline".into(),
        }
    }
}

/// Where the result goes.
#[derive(Clone, Debug)]
pub enum Dest {
    File(PathBuf),
    Dir(PathBuf),
    /// Keep everything in memory (web: handed to a browser download afterwards).
    Memory,
}

#[derive(Default, Debug)]
pub struct ExportOutput {
    /// Populated for [`Dest::Memory`].
    pub files: Vec<(String, Vec<u8>)>,
    pub summary: String,
}

pub enum Step {
    More,
    Done(ExportOutput),
}

/// `Write` into a buffer that can still be read after the encoder consumed the writer.
#[derive(Clone, Default)]
pub struct SharedBuf(pub Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().map_err(|_| std::io::Error::other("poisoned"))?.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SharedBuf {
    pub fn take(&self) -> Vec<u8> {
        self.0.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
    }
}

pub fn sanitize_stem(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "flipline".into()
    } else {
        s
    }
}

/// Frame pixels at export size as tightly packed RGBA8: straight alpha when `transparent`,
/// otherwise flattened over the paper colour.
pub(crate) fn render_rgba(
    f: &FrameSnapshot,
    snap: &Snapshot,
    scale: Scale,
    transparent: bool,
) -> (u32, u32, Vec<u8>) {
    let (ow, oh) = scale.dims(snap.width, snap.height);
    let scaled: Option<Pixmap> = match (&f.pixmap, scale) {
        (None, _) => None,
        (Some(pm), Scale::Full) => Some((**pm).clone()),
        (Some(pm), Scale::Half) => Some(downscale_half(pm)),
    };
    let rgba = if transparent {
        match &scaled {
            Some(pm) => pixmap_to_straight(pm),
            None => vec![0; ow as usize * oh as usize * 4],
        }
    } else {
        flatten_over(scaled.as_ref(), ow, oh, snap.paper)
    };
    (ow, oh, rgba)
}

/// One export in progress.
pub struct ExportJob {
    snap: Snapshot,
    stage: Stage,
    total: usize,
    done: usize,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicUsize>,
}

enum Stage {
    Gif(gif_out::GifStage),
    Png(png_out::PngStage),
    Sheet(sheet::SheetStage),
    Mp4(ffmpeg::Mp4Stage),
}

impl ExportJob {
    pub fn new(snap: Snapshot, opts: ExportOptions, dest: Dest) -> Result<Self, ExportError> {
        if snap.frames.is_empty() {
            return Err(ExportError::Empty);
        }
        let (stage, total) = match opts.format {
            Format::Gif => {
                let s = gif_out::GifStage::new(&snap, &opts, &dest)?;
                let n = snap.frames.len();
                (Stage::Gif(s), n)
            }
            Format::PngSequence => {
                let s = png_out::PngStage::new(&snap, &opts, dest)?;
                let n = s.steps();
                (Stage::Png(s), n)
            }
            Format::SpriteSheet => {
                let s = sheet::SheetStage::new(&snap, &opts, dest)?;
                let n = snap.frames.len() + 1;
                (Stage::Sheet(s), n)
            }
            Format::Mp4 => {
                let s = ffmpeg::Mp4Stage::new(&snap, &opts, &dest)?;
                let n = s.steps();
                (Stage::Mp4(s), n)
            }
        };
        Ok(Self {
            snap,
            stage,
            total,
            done: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn total_steps(&self) -> usize {
        self.total
    }

    pub fn done_steps(&self) -> usize {
        self.done
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub fn progress_counter(&self) -> Arc<AtomicUsize> {
        self.progress.clone()
    }

    /// Does one unit of work (one frame, or the final encode).
    pub fn step(&mut self) -> Result<Step, ExportError> {
        if self.cancel.load(Ordering::Relaxed) {
            self.abort();
            return Err(ExportError::Cancelled);
        }
        let r = match &mut self.stage {
            Stage::Gif(s) => s.step(&self.snap),
            Stage::Png(s) => s.step(&self.snap),
            Stage::Sheet(s) => s.step(&self.snap),
            Stage::Mp4(s) => s.step(&self.snap),
        };
        match r {
            Ok(step) => {
                self.done += 1;
                self.progress.store(self.done, Ordering::Relaxed);
                Ok(step)
            }
            Err(e) => {
                self.abort();
                Err(e)
            }
        }
    }

    fn abort(&mut self) {
        match &mut self.stage {
            Stage::Gif(s) => s.abort(),
            Stage::Png(_) => {}
            Stage::Sheet(_) => {}
            Stage::Mp4(s) => s.abort(),
        }
    }

    /// Runs to completion (used on the worker thread and in tests).
    pub fn run(mut self) -> Result<ExportOutput, ExportError> {
        loop {
            if let Step::Done(out) = self.step()? {
                return Ok(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_filesystem_safe() {
        assert_eq!(sanitize_stem("My Cool/Anim: v2"), "My_Cool_Anim__v2");
        assert_eq!(sanitize_stem("   "), "flipline");
        assert_eq!(sanitize_stem("../../etc"), "etc");
    }

    #[test]
    fn half_scale_dims_never_hit_zero() {
        assert_eq!(Scale::Half.dims(960, 540), (480, 270));
        assert_eq!(Scale::Half.dims(1, 1), (1, 1));
        assert_eq!(Scale::Full.dims(7, 9), (7, 9));
    }
}
