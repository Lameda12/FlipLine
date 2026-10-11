//! Animated GIF: a fresh 256-colour palette per frame (exact when a frame has ≤ 256 colours,
//! NeuQuant otherwise), infinite loop, per-frame delay = hold/fps in centiseconds.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use gif::{DisposalMethod, Encoder, Frame, Repeat};

use super::{render_rgba, Dest, ExportError, ExportOptions, ExportOutput, Scale, SharedBuf, Step};
use crate::model::timing::gif_delays_cs;
use crate::model::Snapshot;

pub struct GifStage {
    enc: Option<Encoder<Box<dyn Write + Send>>>,
    delays: Vec<u16>,
    next: usize,
    scale: Scale,
    transparent: bool,
    mem: Option<SharedBuf>,
    path: Option<PathBuf>,
}

impl GifStage {
    pub fn new(snap: &Snapshot, opts: &ExportOptions, dest: &Dest) -> Result<Self, ExportError> {
        let (w, h) = opts.scale.dims(snap.width, snap.height);
        if w > u16::MAX as u32 || h > u16::MAX as u32 {
            return Err(ExportError::Unsupported(
                "GIF dimensions are limited to 65535 px".into(),
            ));
        }
        let (writer, mem, path): (Box<dyn Write + Send>, _, _) = match dest {
            Dest::File(p) => (Box::new(BufWriter::new(File::create(p)?)), None, Some(p.clone())),
            Dest::Memory => {
                let b = SharedBuf::default();
                (Box::new(b.clone()), Some(b), None)
            }
            Dest::Dir(_) => return Err(ExportError::Unsupported("GIF needs a file destination".into())),
        };
        let mut enc = Encoder::new(writer, w as u16, h as u16, &[])?;
        enc.set_repeat(Repeat::Infinite)?;
        Ok(Self {
            enc: Some(enc),
            delays: gif_delays_cs(&snap.holds(), snap.fps),
            next: 0,
            scale: opts.scale,
            transparent: opts.transparent,
            mem,
            path,
        })
    }

    pub fn step(&mut self, snap: &Snapshot) -> Result<Step, ExportError> {
        let i = self.next;
        let f = &snap.frames[i];
        let (w, h, mut rgba) = render_rgba(f, snap, self.scale, self.transparent);
        if self.transparent {
            // GIF has 1-bit transparency: threshold alpha, keep the straight colour.
            for px in rgba.chunks_exact_mut(4) {
                px[3] = if px[3] >= 128 { 255 } else { 0 };
            }
        }
        let mut frame = Frame::from_rgba_speed(w as u16, h as u16, &mut rgba, 10);
        frame.delay = self.delays[i];
        frame.dispose = if self.transparent {
            DisposalMethod::Background
        } else {
            DisposalMethod::Keep
        };
        let enc = self.enc.as_mut().ok_or(ExportError::Cancelled)?;
        enc.write_frame(&frame)?;
        self.next += 1;
        if self.next < snap.frames.len() {
            return Ok(Step::More);
        }
        // Finish: writes the trailer and flushes.
        let mut w = self.enc.take().ok_or(ExportError::Cancelled)?.into_inner()?;
        w.flush()?;
        let mut out = ExportOutput {
            summary: format!("Exported {} frames to GIF", snap.frames.len()),
            ..Default::default()
        };
        if let Some(m) = &self.mem {
            out.files
                .push((format!("{}.gif", super::sanitize_stem(&snap.name)), m.take()));
        }
        Ok(Step::Done(out))
    }

    pub fn abort(&mut self) {
        self.enc = None;
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{ExportJob, Format};
    use crate::model::pixels::flatten_over;
    use crate::model::Project;
    use tiny_skia::{Color, Paint, PathBuilder, Transform};

    fn flat_project() -> Project {
        let mut p = Project::new("gif test", 64, 48);
        p.fps = 12;
        p.paper = [250, 247, 238];
        let colors = [[220u8, 40, 40], [40, 160, 70], [30, 60, 200]];
        for (i, c) in colors.iter().enumerate() {
            let mut f = p.new_blank_frame();
            // Non-anti-aliased rect keeps each frame to 2 colours: palette is exact.
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(c[0], c[1], c[2], 255));
            paint.anti_alias = false;
            let path = PathBuilder::from_rect(
                tiny_skia::Rect::from_xywh(8.0 + i as f32 * 10.0, 6.0, 20.0, 30.0).unwrap(),
            );
            f.pixmap_mut().fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::identity(),
                None,
            );
            f.set_hold(i as u8 + 1);
            let at = p.len();
            p.insert_frame(at, f);
        }
        p.remove_frame(0); // drop the initial blank frame
        p
    }

    type Decoded = Vec<(Vec<u8>, u16)>;

    fn decode(bytes: &[u8]) -> (gif::Decoder<&[u8]>, Decoded) {
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::RGBA);
        let mut dec = opts.read_info(bytes).unwrap();
        let mut frames = Vec::new();
        while let Some(f) = dec.read_next_frame().unwrap() {
            frames.push((f.buffer.to_vec(), f.delay));
        }
        (dec, frames)
    }

    #[test]
    fn gif_round_trip_matches_pixels_delays_and_loops_forever() {
        let p = flat_project();
        let snap = p.snapshot();
        let mut opts = ExportOptions::new(Format::Gif);
        opts.transparent = false;
        let out = ExportJob::new(snap.clone(), opts, Dest::Memory)
            .unwrap()
            .run()
            .unwrap();
        let bytes = &out.files[0].1;
        assert_eq!(&bytes[0..6], b"GIF89a");

        let (dec, frames) = decode(bytes);
        assert_eq!((dec.width(), dec.height()), (64, 48));
        assert_eq!(frames.len(), 3);

        // Pixel-exact: compare against the same frame flattened over paper.
        for (i, (rgba, _)) in frames.iter().enumerate() {
            let expected = flatten_over(snap.frames[i].pixmap.as_deref(), 64, 48, snap.paper);
            assert_eq!(rgba, &expected, "frame {i} pixels differ after GIF round trip");
        }
        // delay = hold / fps in centiseconds (holds 1,2,3 at 12 fps).
        let delays: Vec<u16> = frames.iter().map(|f| f.1).collect();
        assert_eq!(delays, gif_delays_cs(&[1, 2, 3], 12));
        assert_eq!(delays, vec![8, 17, 25]);

        // NETSCAPE2.0 application extension with loop count 0 = infinite.
        let needle = b"NETSCAPE2.0";
        let pos = bytes
            .windows(needle.len())
            .position(|w| w == needle)
            .expect("loop extension present");
        assert_eq!(
            &bytes[pos + 11..pos + 16],
            &[3, 1, 0, 0, 0],
            "loop count 0 = forever"
        );
    }

    #[test]
    fn many_colour_frames_get_their_own_256_colour_palette() {
        let mut p = Project::new("gradient", 64, 64);
        {
            let f = p.frame_mut(0).unwrap();
            let pm = f.pixmap_mut();
            for (i, px) in pm.data_mut().chunks_exact_mut(4).enumerate() {
                let (x, y) = ((i % 64) as u8, (i / 64) as u8);
                px.copy_from_slice(&[x * 4, y * 4, 255 - x * 2, 255]);
            }
        }
        let snap = p.snapshot();
        let mut opts = ExportOptions::new(Format::Gif);
        opts.transparent = false;
        let out = ExportJob::new(snap.clone(), opts, Dest::Memory)
            .unwrap()
            .run()
            .unwrap();
        let (_, frames) = decode(&out.files[0].1);
        let expected = flatten_over(snap.frames[0].pixmap.as_deref(), 64, 64, snap.paper);
        let total: u64 = frames[0]
            .0
            .chunks(4)
            .zip(expected.chunks(4))
            .map(|(a, b)| (0..3).map(|c| a[c].abs_diff(b[c]) as u64).sum::<u64>())
            .sum();
        let err = total as f64 / (64.0 * 64.0 * 3.0);
        assert!(
            err < 10.0,
            "quantised gradient stays close, mean channel error {err}"
        );
    }

    #[test]
    fn transparent_gif_keeps_empty_pixels_transparent() {
        let p = flat_project();
        let mut opts = ExportOptions::new(Format::Gif);
        opts.transparent = true;
        let out = ExportJob::new(p.snapshot(), opts, Dest::Memory)
            .unwrap()
            .run()
            .unwrap();
        let (_, frames) = decode(&out.files[0].1);
        assert_eq!(frames[0].0[3], 0, "corner pixel transparent");
        let mid = (20 * 64 + 15) * 4;
        assert_eq!(frames[0].0[mid + 3], 255);
    }

    #[test]
    fn half_scale_halves_the_canvas() {
        let p = flat_project();
        let mut opts = ExportOptions::new(Format::Gif);
        opts.scale = Scale::Half;
        let out = ExportJob::new(p.snapshot(), opts, Dest::Memory)
            .unwrap()
            .run()
            .unwrap();
        let (dec, _) = decode(&out.files[0].1);
        assert_eq!((dec.width(), dec.height()), (32, 24));
    }
}
