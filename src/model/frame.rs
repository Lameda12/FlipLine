use std::sync::Arc;

use tiny_skia::Pixmap;

use super::pixels::IRect;
use super::timing::{MAX_HOLD, MIN_HOLD};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameId(pub u64);

/// What changed since the last GPU upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dirty {
    Clean,
    Rect(IRect),
    Full,
}

impl Dirty {
    fn merge(self, r: Option<IRect>) -> Dirty {
        match (self, r) {
            (Dirty::Full, _) | (_, None) => Dirty::Full,
            (Dirty::Clean, Some(r)) => Dirty::Rect(r),
            (Dirty::Rect(a), Some(b)) => Dirty::Rect(a.union(&b)),
        }
    }
}

/// GPU-side caches for a frame. Owned by the frame so they are freed with it, but only
/// `render.rs` touches them.
#[derive(Clone, Default)]
pub struct FrameGpu {
    pub texture: Option<eframe::egui::TextureHandle>,
    pub thumb: Option<eframe::egui::TextureHandle>,
    pub thumb_rev: u64,
}

/// One drawing: a transparent premultiplied-RGBA layer plus its exposure (`hold`).
///
/// The pixmap is `None` until first drawn on, so blank frames cost no memory, and it lives
/// behind an `Arc` so export/autosave snapshots are copy-on-write instead of full copies.
#[derive(Clone)]
pub struct Frame {
    pub id: FrameId,
    width: u32,
    height: u32,
    pixmap: Option<Arc<Pixmap>>,
    pub hold: u8,
    /// Bumped on every pixel change; caches (thumbnail, onion, autosave PNGs) key off it.
    rev: u64,
    dirty: Dirty,
    pub gpu: FrameGpu,
}

impl Frame {
    pub fn blank(id: FrameId, width: u32, height: u32) -> Self {
        Self {
            id,
            width,
            height,
            pixmap: None,
            hold: MIN_HOLD,
            rev: 1,
            dirty: Dirty::Full,
            gpu: FrameGpu::default(),
        }
    }

    pub fn from_pixmap(id: FrameId, pixmap: Pixmap) -> Self {
        let (width, height) = (pixmap.width(), pixmap.height());
        Self {
            id,
            width,
            height,
            pixmap: Some(Arc::new(pixmap)),
            hold: MIN_HOLD,
            rev: 1,
            dirty: Dirty::Full,
            gpu: FrameGpu::default(),
        }
    }

    /// Cheap duplicate: shares pixels copy-on-write, fresh id, no GPU caches.
    pub fn duplicate(&self, id: FrameId) -> Self {
        Self {
            id,
            width: self.width,
            height: self.height,
            pixmap: self.pixmap.clone(),
            hold: self.hold,
            rev: 1,
            dirty: Dirty::Full,
            gpu: FrameGpu::default(),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn is_blank(&self) -> bool {
        self.pixmap.is_none()
    }

    pub fn pixmap(&self) -> Option<&Pixmap> {
        self.pixmap.as_deref()
    }

    pub fn pixmap_arc(&self) -> Option<Arc<Pixmap>> {
        self.pixmap.clone()
    }

    /// Mutable access, allocating the transparent pixmap on first use. The caller must follow up
    /// with [`Frame::touch`] describing what it changed.
    pub fn pixmap_mut(&mut self) -> &mut Pixmap {
        let (w, h) = (self.width, self.height);
        let arc = self
            .pixmap
            .get_or_insert_with(|| Arc::new(Pixmap::new(w, h).expect("frame size is non-zero")));
        Arc::make_mut(arc)
    }

    /// Records a pixel change. `None` means "everything may have changed".
    pub fn touch(&mut self, rect: Option<IRect>) {
        self.rev += 1;
        let rect = rect.map(|r| r.clamp_to(self.width, self.height));
        self.dirty = self.dirty.merge(rect);
    }

    pub fn dirty(&self) -> Dirty {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = Dirty::Clean;
    }

    /// Forgets GPU state so the next draw re-uploads the whole frame.
    pub fn invalidate_gpu(&mut self) {
        self.gpu.texture = None;
        self.dirty = Dirty::Full;
    }

    pub fn set_hold(&mut self, hold: u8) {
        self.hold = hold.clamp(MIN_HOLD, MAX_HOLD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_frames_allocate_lazily_and_duplicates_are_copy_on_write() {
        let mut f = Frame::blank(FrameId(1), 8, 8);
        assert!(f.is_blank());
        f.pixmap_mut().data_mut()[3] = 255;
        f.touch(Some(IRect::new(0, 0, 1, 1)));
        assert!(!f.is_blank());

        let mut d = f.duplicate(FrameId(2));
        assert_eq!(d.pixmap().unwrap().data()[3], 255);
        d.pixmap_mut().data_mut()[3] = 0;
        assert_eq!(f.pixmap().unwrap().data()[3], 255, "original untouched");
    }

    #[test]
    fn dirty_rects_merge_and_full_wins() {
        let mut f = Frame::blank(FrameId(1), 100, 100);
        f.clear_dirty();
        f.touch(Some(IRect::new(1, 1, 5, 5)));
        f.touch(Some(IRect::new(10, 10, 20, 20)));
        assert_eq!(f.dirty(), Dirty::Rect(IRect::new(1, 1, 20, 20)));
        f.touch(None);
        assert_eq!(f.dirty(), Dirty::Full);
    }
}
