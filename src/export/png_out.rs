//! PNG sequence: `name_0001.png`, `name_0002.png`... optionally one image per exposure (holds
//! expanded) so playing the sequence at the project fps reproduces the timing.

use std::path::PathBuf;
use std::sync::Arc;

use super::{render_rgba, sanitize_stem, Dest, ExportError, ExportOptions, ExportOutput, Scale, Step};
use crate::model::io::encode_png_rgba;
use crate::model::Snapshot;

pub struct PngStage {
    dir: Option<PathBuf>,
    base: String,
    order: Vec<usize>,
    next: usize,
    scale: Scale,
    transparent: bool,
    files: Vec<(String, Vec<u8>)>,
    last: Option<(usize, Arc<Vec<u8>>)>,
}

impl PngStage {
    pub fn new(snap: &Snapshot, opts: &ExportOptions, dest: Dest) -> Result<Self, ExportError> {
        let dir = match dest {
            Dest::Dir(d) => {
                std::fs::create_dir_all(&d)?;
                Some(d)
            }
            Dest::Memory => None,
            Dest::File(_) => return Err(ExportError::Unsupported("PNG sequence needs a folder".into())),
        };
        let mut order = Vec::new();
        for (i, f) in snap.frames.iter().enumerate() {
            let repeat = if opts.expand_holds { f.hold as usize } else { 1 };
            order.extend(std::iter::repeat(i).take(repeat));
        }
        Ok(Self {
            dir,
            base: sanitize_stem(&opts.base_name),
            order,
            next: 0,
            scale: opts.scale,
            transparent: opts.transparent,
            files: Vec::new(),
            last: None,
        })
    }

    pub fn steps(&self) -> usize {
        self.order.len()
    }

    pub fn step(&mut self, snap: &Snapshot) -> Result<Step, ExportError> {
        let fi = self.order[self.next];
        let bytes = match &self.last {
            Some((i, b)) if *i == fi => b.clone(),
            _ => {
                let (w, h, rgba) = render_rgba(&snap.frames[fi], snap, self.scale, self.transparent);
                let b = Arc::new(encode_png_rgba(w, h, &rgba).map_err(|e| ExportError::Unsupported(e.to_string()))?);
                self.last = Some((fi, b.clone()));
                b
            }
        };
        let name = format!("{}_{:04}.png", self.base, self.next + 1);
        match &self.dir {
            Some(d) => std::fs::write(d.join(&name), &*bytes)?,
            None => self.files.push((name, (*bytes).clone())),
        }
        self.next += 1;
        if self.next < self.order.len() {
            return Ok(Step::More);
        }
        Ok(Step::Done(ExportOutput {
            files: std::mem::take(&mut self.files),
            summary: format!("Exported {} PNG images", self.order.len()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::{ExportJob, Format};
    use crate::model::Project;

    #[test]
    fn holds_expand_to_one_image_per_exposure_and_names_are_numbered() {
        let mut p = Project::new("my anim", 16, 16);
        let mut f = p.new_blank_frame();
        f.pixmap_mut().data_mut()[3] = 255;
        f.set_hold(3);
        p.insert_frame(1, f);
        p.frame_mut(0).unwrap().set_hold(2);

        let mut opts = ExportOptions::new(Format::PngSequence);
        opts.base_name = "my anim".into();
        let out = ExportJob::new(p.snapshot(), opts.clone(), Dest::Memory).unwrap().run().unwrap();
        let names: Vec<_> = out.files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["my_anim_0001.png", "my_anim_0002.png", "my_anim_0003.png", "my_anim_0004.png", "my_anim_0005.png"]);
        assert_eq!(out.files[0].1, out.files[1].1, "held frames are identical");
        assert_ne!(out.files[1].1, out.files[2].1);

        opts.expand_holds = false;
        let out = ExportJob::new(p.snapshot(), opts, Dest::Memory).unwrap().run().unwrap();
        assert_eq!(out.files.len(), 2);
    }

    #[test]
    fn writes_to_a_directory() {
        let p = Project::new("dir test", 8, 8);
        let dir = std::env::temp_dir().join(format!("flipline-test-{}", std::process::id()));
        let opts = ExportOptions::new(Format::PngSequence);
        ExportJob::new(p.snapshot(), opts, Dest::Dir(dir.clone())).unwrap().run().unwrap();
        let img = image::open(dir.join("flipline_0001.png")).unwrap();
        assert_eq!((img.width(), img.height()), (8, 8));
        let _ = std::fs::remove_dir_all(dir);
    }
}
