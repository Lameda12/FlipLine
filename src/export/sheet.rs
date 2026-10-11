//! Transparent sprite sheet: one cell per drawing in a near-square (sqrt) grid, row-major.

use std::path::PathBuf;

use super::{render_rgba, sanitize_stem, Dest, ExportError, ExportOptions, ExportOutput, Scale, Step};
use crate::model::io::encode_png_rgba;
use crate::model::Snapshot;

const MAX_SIDE: u64 = 16384;
const MAX_BYTES: u64 = 640 * 1024 * 1024;

pub struct SheetStage {
    dest: Dest,
    base: String,
    cols: usize,
    rows: usize,
    cell: (u32, u32),
    scale: Scale,
    sheet: Vec<u8>,
    next: usize,
}

pub fn grid_for(n: usize) -> (usize, usize) {
    let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
    let rows = n.div_ceil(cols).max(1);
    (cols, rows)
}

impl SheetStage {
    pub fn new(snap: &Snapshot, opts: &ExportOptions, dest: Dest) -> Result<Self, ExportError> {
        if matches!(dest, Dest::Dir(_)) {
            return Err(ExportError::Unsupported(
                "sprite sheet needs a file destination".into(),
            ));
        }
        let (cols, rows) = grid_for(snap.frames.len());
        let cell = opts.scale.dims(snap.width, snap.height);
        let (w, h) = (cols as u64 * cell.0 as u64, rows as u64 * cell.1 as u64);
        let bytes = w * h * 4;
        if w > MAX_SIDE || h > MAX_SIDE || bytes > MAX_BYTES {
            return Err(ExportError::SheetTooLarge {
                w,
                h,
                mb: bytes / (1024 * 1024),
            });
        }
        Ok(Self {
            dest,
            base: sanitize_stem(&opts.base_name),
            cols,
            rows,
            cell,
            scale: opts.scale,
            sheet: vec![0; bytes as usize],
            next: 0,
        })
    }

    pub fn step(&mut self, snap: &Snapshot) -> Result<Step, ExportError> {
        let (cw, ch) = (self.cell.0 as usize, self.cell.1 as usize);
        let sheet_w = self.cols * cw;
        if self.next < snap.frames.len() {
            let i = self.next;
            // Straight alpha so the PNG is written without another conversion pass.
            let (_, _, rgba) = render_rgba(&snap.frames[i], snap, self.scale, true);
            let (col, row) = (i % self.cols, i / self.cols);
            for y in 0..ch {
                let dst = ((row * ch + y) * sheet_w + col * cw) * 4;
                self.sheet[dst..dst + cw * 4].copy_from_slice(&rgba[y * cw * 4..(y + 1) * cw * 4]);
            }
            self.next += 1;
            return Ok(Step::More);
        }
        let png = encode_png_rgba(sheet_w as u32, (self.rows * ch) as u32, &self.sheet)
            .map_err(|e| ExportError::Unsupported(e.to_string()))?;
        let mut out = ExportOutput {
            summary: format!(
                "Exported {}×{} sprite sheet ({} frames)",
                self.cols,
                self.rows,
                snap.frames.len()
            ),
            ..Default::default()
        };
        match &self.dest {
            Dest::File(p) => write_file(p, &png)?,
            _ => out.files.push((format!("{}_sheet.png", self.base), png)),
        }
        self.sheet = Vec::new();
        Ok(Step::Done(out))
    }
}

fn write_file(p: &PathBuf, bytes: &[u8]) -> Result<(), ExportError> {
    std::fs::write(p, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{ExportJob, Format};
    use crate::model::Project;

    #[test]
    fn grid_is_roughly_square() {
        assert_eq!(grid_for(1), (1, 1));
        assert_eq!(grid_for(4), (2, 2));
        assert_eq!(grid_for(5), (3, 2));
        assert_eq!(grid_for(12), (4, 3));
        assert_eq!(grid_for(100), (10, 10));
        assert_eq!(grid_for(101), (11, 10));
    }

    #[test]
    fn sheet_places_frames_row_major_on_transparent_background() {
        let mut p = Project::new("sheet", 10, 6);
        for i in 1..5u8 {
            let mut f = p.new_blank_frame();
            for px in f.pixmap_mut().data_mut().chunks_exact_mut(4) {
                px.copy_from_slice(&[i * 40, 0, 0, 255]);
            }
            let at = p.len();
            p.insert_frame(at, f);
        }
        p.remove_frame(0); // frames 1..=4, each a solid red level
        let opts = ExportOptions::new(Format::SpriteSheet);
        let out = ExportJob::new(p.snapshot(), opts, Dest::Memory)
            .unwrap()
            .run()
            .unwrap();
        let img = image::load_from_memory(&out.files[0].1).unwrap().to_rgba8();
        assert_eq!((img.width(), img.height()), (20, 12), "2x2 grid of 10x6 cells");
        assert_eq!(img.get_pixel(0, 0).0, [40, 0, 0, 255]);
        assert_eq!(img.get_pixel(10, 0).0, [80, 0, 0, 255]);
        assert_eq!(img.get_pixel(0, 6).0, [120, 0, 0, 255]);
        assert_eq!(img.get_pixel(19, 11).0, [160, 0, 0, 255]);
    }

    #[test]
    fn unused_cells_stay_transparent() {
        let mut p = Project::new("sheet", 4, 4);
        for _ in 0..2 {
            let mut f = p.new_blank_frame();
            f.pixmap_mut().fill(tiny_skia::Color::from_rgba8(0, 255, 0, 255));
            let at = p.len();
            p.insert_frame(at, f);
        }
        // 3 frames -> 2x2 grid, bottom-right empty.
        let out = ExportJob::new(
            p.snapshot(),
            ExportOptions::new(Format::SpriteSheet),
            Dest::Memory,
        )
        .unwrap()
        .run()
        .unwrap();
        let img = image::load_from_memory(&out.files[0].1).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(7, 7).0[3], 0);
        assert_eq!(
            img.get_pixel(0, 0).0[3],
            0,
            "first frame was blank -> transparent"
        );
        assert_eq!(img.get_pixel(5, 1).0, [0, 255, 0, 255]);
    }

    #[test]
    fn oversized_sheets_are_refused_with_a_hint() {
        let mut p = Project::new("big", 4096, 4096);
        for _ in 0..11 {
            let f = p.new_blank_frame();
            let at = p.len();
            p.insert_frame(at, f);
        }
        let err = ExportJob::new(
            p.snapshot(),
            ExportOptions::new(Format::SpriteSheet),
            Dest::Memory,
        )
        .err()
        .unwrap();
        assert!(matches!(err, ExportError::SheetTooLarge { .. }));
        assert!(err.to_string().contains("50%"));
        let mut half = ExportOptions::new(Format::SpriteSheet);
        half.scale = Scale::Half;
        assert!(ExportJob::new(p.snapshot(), half, Dest::Memory).is_ok());
    }
}
