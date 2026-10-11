//! Drawing tools behind a common [`Tool`] trait. Tools mutate the current frame directly and hand
//! back an undo command when a gesture finishes; the UI never touches pixels itself.

pub mod brush;
pub mod fill;
pub mod picker;
pub mod stroke;

use crate::history::Command;
use crate::model::Frame;

pub const MIN_SIZE: f32 = 1.0;
pub const MAX_SIZE: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolId {
    Brush,
    Eraser,
    Fill,
    Picker,
}

impl ToolId {
    pub const ALL: [ToolId; 4] = [ToolId::Brush, ToolId::Eraser, ToolId::Fill, ToolId::Picker];

    pub fn name(self) -> &'static str {
        match self {
            ToolId::Brush => "Brush",
            ToolId::Eraser => "Eraser",
            ToolId::Fill => "Fill",
            ToolId::Picker => "Picker",
        }
    }

    pub fn key(self) -> char {
        match self {
            ToolId::Brush => 'B',
            ToolId::Eraser => 'E',
            ToolId::Fill => 'G',
            ToolId::Picker => 'I',
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToolSettings {
    pub color: [u8; 3],
    /// Maximum stroke diameter in canvas pixels.
    pub size: f32,
    /// Flood-fill colour tolerance, 0..=255 per channel.
    pub tolerance: u8,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            color: [0x1B, 0x1B, 0x22],
            size: 6.0,
            tolerance: 32,
        }
    }
}

/// One input sample in canvas pixel coordinates. `pressure` is 0..=1; devices without pressure
/// report 1.0 so strokes fall back to constant width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
}

impl Sample {
    pub fn new(x: f32, y: f32, pressure: f32) -> Self {
        Self { x, y, pressure }
    }
}

pub enum ToolOutput {
    None,
    /// A finished gesture, already applied to the frame; push it on the history stack.
    Commit(Box<dyn Command>),
    PickedColor([u8; 3]),
}

pub trait Tool {
    fn id(&self) -> ToolId;
    fn begin(&mut self, frame: &mut Frame, settings: &ToolSettings, at: Sample) -> ToolOutput;
    fn drag(&mut self, frame: &mut Frame, settings: &ToolSettings, at: Sample);
    fn end(&mut self, frame: &mut Frame, settings: &ToolSettings, at: Sample) -> ToolOutput;
    /// Abandons the gesture, restoring any pixels already changed.
    fn cancel(&mut self, frame: &mut Frame);
    fn is_active(&self) -> bool;
}

/// All tools, owned together so the app can dispatch by [`ToolId`].
pub struct Toolbox {
    pub brush: brush::BrushTool,
    pub eraser: brush::EraserTool,
    pub fill: fill::FillTool,
    pub picker: picker::PickerTool,
}

impl Default for Toolbox {
    fn default() -> Self {
        Self {
            brush: brush::BrushTool::default(),
            eraser: brush::EraserTool::default(),
            fill: fill::FillTool,
            picker: picker::PickerTool,
        }
    }
}

impl Toolbox {
    pub fn get(&mut self, id: ToolId) -> &mut dyn Tool {
        match id {
            ToolId::Brush => &mut self.brush,
            ToolId::Eraser => &mut self.eraser,
            ToolId::Fill => &mut self.fill,
            ToolId::Picker => &mut self.picker,
        }
    }
}
