//! Undo/redo as the command pattern. Every undoable edit (strokes, fills, timeline changes)
//! implements [`Command`]; pixel edits store only the dirty rectangle's before/after bytes.

use crate::model::pixels::{write_rect, IRect};
use crate::model::{Frame, FrameId, Project};

pub const MAX_ENTRIES: usize = 50;
/// Soft memory budget for stored pixel data; oldest entries are dropped beyond it.
pub const MAX_BYTES: usize = 192 * 1024 * 1024;

/// An undoable edit. `apply`/`revert` return the frame index the UI should select afterwards.
pub trait Command: Send {
    fn apply(&mut self, project: &mut Project) -> Option<usize>;
    fn revert(&mut self, project: &mut Project) -> Option<usize>;
    fn label(&self) -> &'static str;
    /// Approximate heap bytes held, for the history budget.
    fn bytes(&self) -> usize {
        0
    }
}

/// Before/after pixels of the rectangle a stroke or fill touched.
pub struct PixelEdit {
    pub frame: FrameId,
    pub rect: IRect,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub label: &'static str,
}

impl PixelEdit {
    fn write(&self, project: &mut Project, data: &[u8]) -> Option<usize> {
        let idx = project.index_of(self.frame)?;
        let frame = project.frame_mut(idx)?;
        write_rect(frame.pixmap_mut(), self.rect, data);
        frame.touch(Some(self.rect));
        Some(idx)
    }
}

impl Command for PixelEdit {
    fn apply(&mut self, project: &mut Project) -> Option<usize> {
        let data = std::mem::take(&mut self.after);
        let r = self.write(project, &data);
        self.after = data;
        r
    }

    fn revert(&mut self, project: &mut Project) -> Option<usize> {
        let data = std::mem::take(&mut self.before);
        let r = self.write(project, &data);
        self.before = data;
        r
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn bytes(&self) -> usize {
        self.before.len() + self.after.len()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SpanMode {
    Insert,
    Remove,
}

/// Inserts or removes a run of frames. The frames live in `stash` whenever they are not in the
/// project, so undoing a delete restores the exact pixels without copies.
pub struct FrameSpan {
    index: usize,
    count: usize,
    stash: Vec<Frame>,
    mode: SpanMode,
    label: &'static str,
}

impl FrameSpan {
    pub fn insert(index: usize, frames: Vec<Frame>, label: &'static str) -> Self {
        Self {
            index,
            count: frames.len(),
            stash: frames,
            mode: SpanMode::Insert,
            label,
        }
    }

    pub fn remove(index: usize, count: usize) -> Self {
        Self {
            index,
            count,
            stash: Vec::new(),
            mode: SpanMode::Remove,
            label: "Delete frame",
        }
    }

    fn put(&mut self, project: &mut Project) -> Option<usize> {
        for (i, mut f) in self.stash.drain(..).enumerate() {
            f.invalidate_gpu();
            project.insert_frame(self.index + i, f);
        }
        Some(self.index.min(project.len().saturating_sub(1)))
    }

    fn take(&mut self, project: &mut Project) -> Option<usize> {
        for _ in 0..self.count {
            if self.index < project.len() {
                if let Some(f) = project.remove_frame(self.index) {
                    self.stash.push(f);
                }
            }
        }
        // Keep the stash in project order (removed front to back).
        Some(
            self.index
                .saturating_sub(usize::from(self.mode == SpanMode::Insert))
                .min(project.len().saturating_sub(1)),
        )
    }
}

impl Command for FrameSpan {
    fn apply(&mut self, project: &mut Project) -> Option<usize> {
        match self.mode {
            SpanMode::Insert => self.put(project),
            SpanMode::Remove => self.take(project),
        }
    }

    fn revert(&mut self, project: &mut Project) -> Option<usize> {
        match self.mode {
            SpanMode::Insert => self.take(project),
            SpanMode::Remove => self.put(project),
        }
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn bytes(&self) -> usize {
        self.stash
            .iter()
            .filter(|f| !f.is_blank())
            .map(|f| f.width() as usize * f.height() as usize * 4)
            .sum()
    }
}

pub struct MoveFrame {
    pub from: usize,
    pub to: usize,
}

impl Command for MoveFrame {
    fn apply(&mut self, project: &mut Project) -> Option<usize> {
        project.move_frame(self.from, self.to);
        Some(self.to)
    }

    fn revert(&mut self, project: &mut Project) -> Option<usize> {
        project.move_frame(self.to, self.from);
        Some(self.from)
    }

    fn label(&self) -> &'static str {
        "Move frame"
    }
}

pub struct SetHold {
    pub frame: FrameId,
    pub old: u8,
    pub new: u8,
}

impl Command for SetHold {
    fn apply(&mut self, project: &mut Project) -> Option<usize> {
        let idx = project.index_of(self.frame)?;
        project.frame_mut(idx)?.set_hold(self.new);
        Some(idx)
    }

    fn revert(&mut self, project: &mut Project) -> Option<usize> {
        let idx = project.index_of(self.frame)?;
        project.frame_mut(idx)?.set_hold(self.old);
        Some(idx)
    }

    fn label(&self) -> &'static str {
        "Change hold"
    }
}

/// Several commands undone/redone as one step.
pub struct Composite {
    pub parts: Vec<Box<dyn Command>>,
    pub label: &'static str,
}

impl Command for Composite {
    fn apply(&mut self, project: &mut Project) -> Option<usize> {
        let mut sel = None;
        for c in self.parts.iter_mut() {
            sel = c.apply(project).or(sel);
        }
        sel
    }

    fn revert(&mut self, project: &mut Project) -> Option<usize> {
        let mut sel = None;
        for c in self.parts.iter_mut().rev() {
            sel = c.revert(project).or(sel);
        }
        sel
    }

    fn label(&self) -> &'static str {
        self.label
    }

    fn bytes(&self) -> usize {
        self.parts.iter().map(|c| c.bytes()).sum()
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<Box<dyn Command>>,
    redo: Vec<Box<dyn Command>>,
    bytes: usize,
    /// Increments on every change, so the app can tell the project is modified.
    generation: u64,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|c| c.label())
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|c| c.label())
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
        self.generation += 1;
    }

    /// Records a command whose effect has already been applied (strokes, fills).
    pub fn push_applied(&mut self, cmd: Box<dyn Command>) {
        self.redo.clear();
        self.bytes += cmd.bytes();
        self.undo.push(cmd);
        self.generation += 1;
        self.trim();
    }

    /// Applies `cmd` and records it. Returns the frame index to select.
    pub fn execute(&mut self, mut cmd: Box<dyn Command>, project: &mut Project) -> Option<usize> {
        let sel = cmd.apply(project);
        self.push_applied(cmd);
        sel
    }

    pub fn undo(&mut self, project: &mut Project) -> Option<usize> {
        let mut cmd = self.undo.pop()?;
        let sel = cmd.revert(project);
        self.redo.push(cmd);
        self.generation += 1;
        sel.or(Some(0))
    }

    pub fn redo(&mut self, project: &mut Project) -> Option<usize> {
        let mut cmd = self.redo.pop()?;
        let sel = cmd.apply(project);
        self.undo.push(cmd);
        self.generation += 1;
        sel.or(Some(0))
    }

    fn trim(&mut self) {
        // `bytes` only tracks the undo stack at push time; recompute cheaply when trimming.
        while self.undo.len() > MAX_ENTRIES || (self.bytes > MAX_BYTES && self.undo.len() > 1) {
            let dropped = self.undo.remove(0);
            self.bytes = self.bytes.saturating_sub(dropped.bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::pixels::extract_rect;

    fn pixel_edit(p: &mut Project, idx: usize, rect: IRect, value: u8) -> Box<dyn Command> {
        let frame = p.frame_mut(idx).unwrap();
        let id = frame.id;
        let before = extract_rect(frame.pixmap_mut(), rect);
        let after = vec![value; rect.area_bytes()];
        write_rect(frame.pixmap_mut(), rect, &after);
        frame.touch(Some(rect));
        Box::new(PixelEdit {
            frame: id,
            rect,
            before,
            after,
            label: "Stroke",
        })
    }

    #[test]
    fn undo_redo_restores_exact_pixels() {
        let mut p = Project::new("t", 64, 64);
        let mut h = History::new();
        // Seed non-trivial content so "before" is not all zeros.
        for (i, b) in p
            .frame_mut(0)
            .unwrap()
            .pixmap_mut()
            .data_mut()
            .iter_mut()
            .enumerate()
        {
            *b = (i * 7 % 251) as u8;
        }
        let original = p.frame(0).unwrap().pixmap().unwrap().data().to_vec();

        let c1 = pixel_edit(&mut p, 0, IRect::new(5, 5, 20, 30), 200);
        h.push_applied(c1);
        let after1 = p.frame(0).unwrap().pixmap().unwrap().data().to_vec();
        let c2 = pixel_edit(&mut p, 0, IRect::new(10, 0, 50, 12), 77);
        h.push_applied(c2);
        let after2 = p.frame(0).unwrap().pixmap().unwrap().data().to_vec();
        assert_ne!(after1, after2);

        assert_eq!(h.undo(&mut p), Some(0));
        assert_eq!(p.frame(0).unwrap().pixmap().unwrap().data(), &after1[..]);
        assert_eq!(h.undo(&mut p), Some(0));
        assert_eq!(p.frame(0).unwrap().pixmap().unwrap().data(), &original[..]);
        assert!(!h.can_undo());
        h.redo(&mut p);
        h.redo(&mut p);
        assert_eq!(p.frame(0).unwrap().pixmap().unwrap().data(), &after2[..]);
        assert!(!h.can_redo());
    }

    #[test]
    fn pixel_edits_store_only_the_dirty_rect() {
        let mut p = Project::new("t", 1280, 720);
        let rect = IRect::new(100, 100, 140, 130);
        let c = pixel_edit(&mut p, 0, rect, 9);
        assert_eq!(c.bytes(), 2 * 40 * 30 * 4);
    }

    #[test]
    fn timeline_edits_share_the_stack_and_undo_in_order() {
        let mut p = Project::new("t", 16, 16);
        let mut h = History::new();
        let new = p.new_blank_frame();
        let new_id = new.id;
        h.execute(Box::new(FrameSpan::insert(1, vec![new], "New frame")), &mut p);
        assert_eq!(p.len(), 2);
        let first = p.frame(0).unwrap().id;
        h.execute(Box::new(MoveFrame { from: 1, to: 0 }), &mut p);
        assert_eq!(p.frame(0).unwrap().id, new_id);
        h.execute(
            Box::new(SetHold {
                frame: first,
                old: 1,
                new: 4,
            }),
            &mut p,
        );
        assert_eq!(p.frame(1).unwrap().hold, 4);
        let pe = pixel_edit(&mut p, 1, IRect::new(0, 0, 4, 4), 255);
        h.push_applied(pe);

        h.undo(&mut p); // pixels
        h.undo(&mut p); // hold
        assert_eq!(p.frame(1).unwrap().hold, 1);
        h.undo(&mut p); // move
        assert_eq!(p.frame(0).unwrap().id, first);
        h.undo(&mut p); // insert
        assert_eq!(p.len(), 1);
        h.redo(&mut p);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn deleting_a_frame_and_undoing_restores_its_pixels() {
        let mut p = Project::new("t", 16, 16);
        let mut h = History::new();
        let extra = p.new_blank_frame();
        p.insert_frame(1, extra);
        p.frame_mut(1).unwrap().pixmap_mut().data_mut()[0] = 42;
        let id = p.frame(1).unwrap().id;

        h.execute(Box::new(FrameSpan::remove(1, 1)), &mut p);
        assert_eq!(p.len(), 1);
        h.undo(&mut p);
        assert_eq!(p.len(), 2);
        assert_eq!(p.frame(1).unwrap().id, id);
        assert_eq!(p.frame(1).unwrap().pixmap().unwrap().data()[0], 42);
    }

    #[test]
    fn history_is_capped_and_new_edits_clear_redo() {
        let mut p = Project::new("t", 16, 16);
        let mut h = History::new();
        for i in 0..(MAX_ENTRIES + 20) {
            let c = pixel_edit(&mut p, 0, IRect::new(0, 0, 2, 2), (i % 200) as u8);
            h.push_applied(c);
        }
        assert_eq!(h.len(), MAX_ENTRIES);
        h.undo(&mut p);
        assert!(h.can_redo());
        let c = pixel_edit(&mut p, 0, IRect::new(0, 0, 2, 2), 1);
        h.push_applied(c);
        assert!(!h.can_redo());
    }

    #[test]
    fn byte_budget_drops_oldest() {
        let mut p = Project::new("t", 2048, 2048);
        let mut h = History::new();
        let rect = IRect::new(0, 0, 2048, 2048); // 16 MiB before + 16 MiB after each
        for i in 0..8 {
            let c = pixel_edit(&mut p, 0, rect, i + 1);
            h.push_applied(c);
        }
        assert!(
            h.len() < 8,
            "oldest full-frame edits are evicted by the byte budget"
        );
        assert!(!h.is_empty());
    }
}
