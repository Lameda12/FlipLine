use super::stroke::StrokeEngine;
use super::{Sample, Tool, ToolId, ToolOutput, ToolSettings};
use crate::model::Frame;

macro_rules! stroke_tool {
    ($name:ident, $id:expr, $erase:expr) => {
        pub struct $name {
            engine: StrokeEngine,
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    engine: StrokeEngine::new($erase),
                }
            }
        }

        impl Tool for $name {
            fn id(&self) -> ToolId {
                $id
            }

            fn begin(&mut self, frame: &mut Frame, s: &ToolSettings, at: Sample) -> ToolOutput {
                self.engine.begin(frame, s, at);
                ToolOutput::None
            }

            fn drag(&mut self, frame: &mut Frame, _s: &ToolSettings, at: Sample) {
                self.engine.drag(frame, at);
            }

            fn end(&mut self, frame: &mut Frame, _s: &ToolSettings, at: Sample) -> ToolOutput {
                match self.engine.end(frame, at) {
                    Some(edit) => ToolOutput::Commit(Box::new(edit)),
                    None => ToolOutput::None,
                }
            }

            fn cancel(&mut self, frame: &mut Frame) {
                self.engine.cancel(frame);
            }

            fn is_active(&self) -> bool {
                self.engine.is_active()
            }
        }
    };
}

stroke_tool!(BrushTool, ToolId::Brush, false);
stroke_tool!(EraserTool, ToolId::Eraser, true);
