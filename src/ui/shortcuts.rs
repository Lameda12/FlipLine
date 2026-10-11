//! Keyboard shortcuts. Combos are consumed before plain keys so Cmd+E (export) never also
//! selects the eraser, and nothing fires while a text field has focus.

use eframe::egui::{Context, Event, Key, Modifiers};

use crate::app::FlipLineApp;
use crate::tools::ToolId;

pub fn handle(app: &mut FlipLineApp, ctx: &Context) {
    // Space is both "play" and "hold to pan": a tap plays, a drag pans.
    if !ctx.egui_wants_keyboard_input() {
        let events = ctx.input(|i| i.events.clone());
        let now = ctx.input(|i| i.time);
        for ev in events {
            if let Event::Key {
                key: Key::Space,
                pressed,
                repeat,
                ..
            } = ev
            {
                if pressed && !repeat {
                    app.input.space_down = true;
                    app.input.space_used = false;
                } else if !pressed {
                    if app.input.space_down && !app.input.space_used {
                        app.toggle_playback(now);
                    }
                    app.input.space_down = false;
                }
            }
        }
    } else {
        app.input.space_down = false;
    }
    if ctx.egui_wants_keyboard_input() {
        return;
    }

    // Navigation, hold, size and undo keys may auto-repeat; actions that create or destroy things
    // (new/duplicate/delete frame, tool switches, save, export...) fire once per physical press.
    let fresh: Vec<Key> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    ..
                } => Some(*key),
                _ => None,
            })
            .collect()
    });
    let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
    let once = |m: Modifiers, key: Key| k(m, key) && fresh.contains(&key);
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;

    if k(cmd_shift, Key::Z) || k(cmd, Key::Y) {
        app.redo();
    }
    if k(cmd, Key::Z) {
        app.undo();
    }
    if once(cmd_shift, Key::S) {
        app.save(ctx, true);
    }
    if once(cmd, Key::S) {
        app.save(ctx, false);
    }
    if once(cmd, Key::E) {
        app.open_export_dialog();
    }
    if k(Modifiers::ALT, Key::ArrowLeft) {
        app.move_frame_by(-1);
    }
    if k(Modifiers::ALT, Key::ArrowRight) {
        app.move_frame_by(1);
    }

    let plain = Modifiers::NONE;
    if once(plain, Key::B) {
        app.set_tool(ToolId::Brush);
    }
    if once(plain, Key::E) {
        app.set_tool(ToolId::Eraser);
    }
    if once(plain, Key::G) {
        app.set_tool(ToolId::Fill);
    }
    if once(plain, Key::I) {
        app.set_tool(ToolId::Picker);
    }
    if k(plain, Key::OpenBracket) {
        app.adjust_size(false);
    }
    if k(plain, Key::CloseBracket) {
        app.adjust_size(true);
    }
    if k(plain, Key::Comma) || k(plain, Key::ArrowLeft) {
        app.step_frame(-1);
    }
    if k(plain, Key::Period) || k(plain, Key::ArrowRight) {
        app.step_frame(1);
    }
    if once(plain, Key::N) {
        app.new_frame();
    }
    if once(plain, Key::D) {
        app.duplicate_frame();
    }
    if once(plain, Key::Delete) || once(plain, Key::Backspace) {
        app.delete_frame();
    }
    if k(plain, Key::Minus) {
        app.change_hold(-1);
    }
    if k(plain, Key::Equals)
        || k(Modifiers::SHIFT, Key::Equals)
        || k(plain, Key::Plus)
        || k(Modifiers::SHIFT, Key::Plus)
    {
        app.change_hold(1);
    }
    if once(plain, Key::O) {
        app.onion.enabled = !app.onion.enabled;
    }
    if once(plain, Key::F) {
        app.view.request = Some(crate::app::ZoomRequest::Fit);
    }
    if once(plain, Key::Num1) {
        app.view.request = Some(crate::app::ZoomRequest::Actual);
    }
    if k(plain, Key::F1)
        || k(Modifiers::SHIFT, Key::Slash)
        || k(plain, Key::Questionmark)
        || k(Modifiers::SHIFT, Key::Questionmark)
    {
        app.ui.help = !app.ui.help;
    }
    if k(plain, Key::Escape) {
        if app.ui.help {
            app.ui.help = false;
        } else if app.stroke.is_some() {
            app.cancel_stroke();
        }
    }
}
