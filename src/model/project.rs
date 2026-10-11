use std::sync::Arc;

use tiny_skia::Pixmap;

use super::frame::{Frame, FrameId};
use super::timing;

pub const SIZE_PRESETS: [(&str, u32, u32); 5] = [
    ("960×540", 960, 540),
    ("1280×720", 1280, 720),
    ("1080×1080", 1080, 1080),
    ("720×1280", 720, 1280),
    ("640×480", 640, 480),
];

pub const DEFAULT_PAPER: [u8; 3] = [0xFA, 0xF7, 0xEE];
pub const MAX_FRAMES: usize = 999;
pub const MAX_DIMENSION: u32 = 4096;

pub struct Project {
    pub name: String,
    width: u32,
    height: u32,
    pub fps: u32,
    pub paper: [u8; 3],
    frames: Vec<Frame>,
    next_id: u64,
}

/// Cheap, thread-safe view of a frame for background jobs (pixels are shared copy-on-write).
#[derive(Clone)]
pub struct FrameSnapshot {
    pub id: FrameId,
    pub rev: u64,
    pub hold: u8,
    pub pixmap: Option<Arc<Pixmap>>,
}

#[derive(Clone)]
pub struct Snapshot {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub paper: [u8; 3],
    pub frames: Vec<FrameSnapshot>,
}

impl Snapshot {
    pub fn holds(&self) -> Vec<u8> {
        self.frames.iter().map(|f| f.hold).collect()
    }
}

impl Project {
    /// A new project with one blank frame.
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        let mut p = Self::empty(name, width, height);
        let f = p.new_blank_frame();
        p.frames.push(f);
        p
    }

    /// A project with no frames, for loaders/generators to fill (callers must add ≥ 1 frame).
    pub fn empty(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            name: name.into(),
            width: width.clamp(1, MAX_DIMENSION),
            height: height.clamp(1, MAX_DIMENSION),
            fps: 12,
            paper: DEFAULT_PAPER,
            frames: Vec::new(),
            next_id: 1,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn alloc_id(&mut self) -> FrameId {
        let id = FrameId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn new_blank_frame(&mut self) -> Frame {
        let id = self.alloc_id();
        Frame::blank(id, self.width, self.height)
    }

    pub fn frame_from_pixmap(&mut self, pm: Pixmap) -> Frame {
        let id = self.alloc_id();
        Frame::from_pixmap(id, pm)
    }

    pub fn duplicate_frame(&mut self, idx: usize) -> Option<Frame> {
        let id = self.alloc_id();
        self.frames.get(idx).map(|f| f.duplicate(id))
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    pub fn frames_mut(&mut self) -> &mut [Frame] {
        &mut self.frames
    }

    pub fn frame(&self, idx: usize) -> Option<&Frame> {
        self.frames.get(idx)
    }

    pub fn frame_mut(&mut self, idx: usize) -> Option<&mut Frame> {
        self.frames.get_mut(idx)
    }

    pub fn index_of(&self, id: FrameId) -> Option<usize> {
        self.frames.iter().position(|f| f.id == id)
    }

    pub fn frame_by_id_mut(&mut self, id: FrameId) -> Option<&mut Frame> {
        self.frames.iter_mut().find(|f| f.id == id)
    }

    /// Inserts at `idx` (clamped to the end).
    pub fn insert_frame(&mut self, idx: usize, frame: Frame) {
        let idx = idx.min(self.frames.len());
        self.frames.insert(idx, frame);
    }

    pub fn remove_frame(&mut self, idx: usize) -> Option<Frame> {
        if idx < self.frames.len() {
            Some(self.frames.remove(idx))
        } else {
            None
        }
    }

    pub fn move_frame(&mut self, from: usize, to: usize) {
        if from < self.frames.len() && to < self.frames.len() && from != to {
            let f = self.frames.remove(from);
            self.frames.insert(to, f);
        }
    }

    pub fn holds(&self) -> Vec<u8> {
        self.frames.iter().map(|f| f.hold).collect()
    }

    pub fn total_ticks(&self) -> u32 {
        self.frames.iter().map(|f| f.hold as u32).sum()
    }

    pub fn duration_secs(&self) -> f64 {
        timing::ticks_to_secs(self.total_ticks(), self.fps)
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            name: self.name.clone(),
            width: self.width,
            height: self.height,
            fps: self.fps,
            paper: self.paper,
            frames: self
                .frames
                .iter()
                .map(|f| FrameSnapshot {
                    id: f.id,
                    rev: f.rev(),
                    hold: f.hold,
                    pixmap: f.pixmap_arc(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_project_has_one_blank_frame() {
        let p = Project::new("t", 640, 480);
        assert_eq!(p.len(), 1);
        assert!(p.frame(0).unwrap().is_blank());
    }

    #[test]
    fn move_and_remove_keep_ids_stable() {
        let mut p = Project::new("t", 16, 16);
        for _ in 0..3 {
            let f = p.new_blank_frame();
            p.insert_frame(p.len(), f);
        }
        let ids: Vec<_> = p.frames().iter().map(|f| f.id).collect();
        p.move_frame(0, 3);
        assert_eq!(p.frame(3).unwrap().id, ids[0]);
        assert_eq!(p.frame(0).unwrap().id, ids[1]);
        let removed = p.remove_frame(0).unwrap();
        assert_eq!(removed.id, ids[1]);
        assert_eq!(p.index_of(ids[0]), Some(2));
    }

    #[test]
    fn snapshot_shares_pixels_copy_on_write() {
        let mut p = Project::new("t", 4, 4);
        p.frame_mut(0).unwrap().pixmap_mut().data_mut()[0] = 9;
        let snap = p.snapshot();
        p.frame_mut(0).unwrap().pixmap_mut().data_mut()[0] = 1;
        assert_eq!(snap.frames[0].pixmap.as_ref().unwrap().data()[0], 9);
    }
}
