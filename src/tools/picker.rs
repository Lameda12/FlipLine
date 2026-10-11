use super::{Sample, Tool, ToolId, ToolOutput, ToolSettings};
use crate::model::pixels::demul_u8;
use crate::model::Frame;

/// Samples the ink colour under the cursor from the current frame.
pub struct PickerTool;

pub fn pick(frame: &Frame, x: i32, y: i32) -> Option<[u8; 3]> {
    if x < 0 || y < 0 {
        return None;
    }
    let px = frame.pixmap()?.pixel(x as u32, y as u32)?;
    let a = px.alpha();
    if a == 0 {
        return None;
    }
    Some([
        demul_u8(px.red(), a),
        demul_u8(px.green(), a),
        demul_u8(px.blue(), a),
    ])
}

impl Tool for PickerTool {
    fn id(&self) -> ToolId {
        ToolId::Picker
    }

    fn begin(&mut self, frame: &mut Frame, _s: &ToolSettings, at: Sample) -> ToolOutput {
        match pick(frame, at.x.floor() as i32, at.y.floor() as i32) {
            Some(c) => ToolOutput::PickedColor(c),
            None => ToolOutput::None,
        }
    }

    fn drag(&mut self, _f: &mut Frame, _s: &ToolSettings, _at: Sample) {}

    fn end(&mut self, _f: &mut Frame, _s: &ToolSettings, _at: Sample) -> ToolOutput {
        ToolOutput::None
    }

    fn cancel(&mut self, _f: &mut Frame) {}

    fn is_active(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FrameId;

    #[test]
    fn picks_straight_colour_from_partially_transparent_pixels() {
        let mut f = Frame::blank(FrameId(1), 4, 4);
        // Premultiplied (100, 50, 0) at alpha 200 -> straight ≈ (127, 64, 0).
        f.pixmap_mut().data_mut()[0..4].copy_from_slice(&[100, 50, 0, 200]);
        let c = pick(&f, 0, 0).unwrap();
        assert_eq!(c, [128, 64, 0]);
        assert!(pick(&f, 1, 0).is_none(), "transparent pixels pick nothing");
        assert!(pick(&f, -1, 0).is_none());
        assert!(pick(&f, 99, 99).is_none());
    }
}
