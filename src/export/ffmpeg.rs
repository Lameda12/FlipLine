//! MP4 by piping raw RGBA frames into `ffmpeg` (native only; needs ffmpeg on PATH).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::{render_rgba, Dest, ExportError, ExportOptions, ExportOutput, Scale, Step};
use crate::model::Snapshot;

#[cfg(not(target_arch = "wasm32"))]
pub fn ffmpeg_available() -> bool {
    quiet(Command::new("ffmpeg"))
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_arch = "wasm32")]
pub fn ffmpeg_available() -> bool {
    false
}

/// Keeps a console window from flashing up when a GUI app spawns ffmpeg on Windows.
#[cfg_attr(not(windows), allow(unused_mut))]
fn quiet(mut cmd: Command) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// The ffmpeg invocation for raw RGBA on stdin. Pure so it can be unit-tested.
pub fn ffmpeg_command(w: u32, h: u32, fps: u32, out: &std::path::Path) -> Vec<String> {
    [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &format!("{w}x{h}"),
        "-framerate",
        &fps.to_string(),
        "-i",
        "-",
        // yuv420p needs even dimensions.
        "-vf",
        "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p",
        "-c:v",
        "libx264",
        "-preset",
        "medium",
        "-crf",
        "17",
        "-movflags",
        "+faststart",
        &out.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub struct Mp4Stage {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<()>>,
    err_text: Arc<Mutex<String>>,
    order: Vec<usize>,
    next: usize,
    scale: Scale,
    last: Option<(usize, Vec<u8>)>,
    path: PathBuf,
}

impl Mp4Stage {
    pub fn new(snap: &Snapshot, opts: &ExportOptions, dest: &Dest) -> Result<Self, ExportError> {
        if cfg!(target_arch = "wasm32") {
            return Err(ExportError::Unsupported(
                "MP4 export needs the desktop app".into(),
            ));
        }
        let Dest::File(path) = dest else {
            return Err(ExportError::Unsupported("MP4 needs a file destination".into()));
        };
        let (w, h) = opts.scale.dims(snap.width, snap.height);
        let mut child = quiet(Command::new("ffmpeg"))
            .args(ffmpeg_command(w, h, snap.fps, path))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ExportError::Ffmpeg(format!("could not start ffmpeg ({e}); is it on your PATH?")))?;
        let stdin = child.stdin.take();
        let err_text = Arc::new(Mutex::new(String::new()));
        let stderr = child.stderr.take().map(|mut s| {
            let sink = err_text.clone();
            std::thread::spawn(move || {
                let mut buf = String::new();
                let _ = std::io::Read::read_to_string(&mut s, &mut buf);
                if let Ok(mut g) = sink.lock() {
                    *g = buf;
                }
            })
        });
        let mut order = Vec::new();
        for (i, f) in snap.frames.iter().enumerate() {
            order.extend(std::iter::repeat_n(i, f.hold as usize));
        }
        Ok(Self {
            child: Some(child),
            stdin,
            stderr,
            err_text,
            order,
            next: 0,
            scale: opts.scale,
            last: None,
            path: path.clone(),
        })
    }

    pub fn steps(&self) -> usize {
        self.order.len()
    }

    fn failure(&mut self, fallback: &str) -> ExportError {
        self.stdin = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.wait();
        }
        if let Some(t) = self.stderr.take() {
            let _ = t.join();
        }
        let text = self
            .err_text
            .lock()
            .map(|g| g.trim().to_string())
            .unwrap_or_default();
        let _ = std::fs::remove_file(&self.path);
        ExportError::Ffmpeg(if text.is_empty() {
            fallback.to_string()
        } else {
            text
        })
    }

    pub fn step(&mut self, snap: &Snapshot) -> Result<Step, ExportError> {
        let fi = self.order[self.next];
        let rebuild = !matches!(&self.last, Some((i, _)) if *i == fi);
        if rebuild {
            let (_, _, rgba) = render_rgba(&snap.frames[fi], snap, self.scale, false);
            self.last = Some((fi, rgba));
        }
        let data = &self.last.as_ref().map(|l| l.1.as_slice()).unwrap_or_default();
        let write = match self.stdin.as_mut() {
            Some(s) => s.write_all(data),
            None => Err(std::io::Error::other("ffmpeg stdin closed")),
        };
        if let Err(e) = write {
            return Err(self.failure(&format!("ffmpeg stopped accepting frames ({e})")));
        }
        self.next += 1;
        if self.next < self.order.len() {
            return Ok(Step::More);
        }
        self.stdin = None; // EOF tells ffmpeg to finalise the file
        let status = self.child.take().map(|mut c| c.wait());
        if let Some(t) = self.stderr.take() {
            let _ = t.join();
        }
        match status {
            Some(Ok(s)) if s.success() => Ok(Step::Done(ExportOutput {
                summary: format!("Exported {} video frames to MP4", self.order.len()),
                ..Default::default()
            })),
            _ => {
                let text = self
                    .err_text
                    .lock()
                    .map(|g| g.trim().to_string())
                    .unwrap_or_default();
                let _ = std::fs::remove_file(&self.path);
                Err(ExportError::Ffmpeg(if text.is_empty() {
                    "ffmpeg failed".into()
                } else {
                    text
                }))
            }
        }
    }

    pub fn abort(&mut self) {
        self.stdin = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{ExportJob, Format};
    use crate::model::Project;

    #[test]
    fn command_line_pipes_rgba_with_even_dimension_guard() {
        let args = ffmpeg_command(960, 540, 24, std::path::Path::new("out.mp4"));
        let joined = args.join(" ");
        assert!(joined.contains("-f rawvideo -pix_fmt rgba -s 960x540 -framerate 24 -i -"));
        assert!(joined.contains("trunc(iw/2)*2"));
        assert!(joined.ends_with("out.mp4"));
    }

    #[test]
    fn mp4_export_produces_a_playable_file_when_ffmpeg_exists() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let mut p = Project::new("vid", 64, 48);
        p.fps = 12;
        p.frame_mut(0).unwrap().set_hold(2);
        let f = p.new_blank_frame();
        p.insert_frame(1, f);
        let path = std::env::temp_dir().join(format!("flipline-test-{}.mp4", std::process::id()));
        let res = ExportJob::new(
            p.snapshot(),
            ExportOptions::new(Format::Mp4),
            Dest::File(path.clone()),
        )
        .unwrap()
        .run();
        match res {
            Ok(_) => {
                let len = std::fs::metadata(&path).unwrap().len();
                assert!(len > 100, "mp4 has content");
                let _ = std::fs::remove_file(&path);
            }
            Err(ExportError::Ffmpeg(msg)) if msg.contains("libx264") || msg.contains("Unknown encoder") => {
                eprintln!("ffmpeg lacks libx264; skipping: {msg}");
            }
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn missing_ffmpeg_binary_is_a_friendly_error() {
        // Exercise the error mapping without depending on the host: bad destination dir.
        if !ffmpeg_available() {
            return;
        }
        let p = Project::new("vid", 16, 16);
        let bad = std::path::PathBuf::from("/nonexistent-dir-xyz/out.mp4");
        let err = ExportJob::new(p.snapshot(), ExportOptions::new(Format::Mp4), Dest::File(bad))
            .and_then(|j| j.run());
        assert!(err.is_err());
    }
}
