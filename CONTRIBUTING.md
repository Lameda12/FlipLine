# Contributing to FlipLine

Thanks for helping. FlipLine is small on purpose: the goal is the core loop animators actually use (draw a frame, flip, onion skin, time it, export), done well and fast.

## Setup

You need stable Rust (1.95 or newer). No system libraries are required to build.

```sh
git clone https://github.com/Lameda12/FlipLine
cd FlipLine
cargo run --release          # desktop app
```

Web build (needs `rustup target add wasm32-unknown-unknown` and [trunk](https://trunkrs.dev)):

```sh
trunk serve --release        # http://127.0.0.1:8080
```

MP4 export needs `ffmpeg` on your `PATH`; the option hides itself when it isn't found.

## Before you open a PR

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo clippy --target wasm32-unknown-unknown --lib --bins -- -D warnings
```

CI runs the same on Linux, macOS and Windows, plus the wasm build.

## How the code is organised

| Path | Responsibility |
| --- | --- |
| `src/model/` | Pure data: `Project`, `Frame` (lazy pixmap, hold, dirty flag, GPU handles), hold timing math, `.flipline` (de)serialisation. No UI. |
| `src/tools/` | `Tool` trait plus brush, eraser, flood fill, picker. Tools mutate the current frame and return an undo command. |
| `src/history.rs` | Command pattern (`apply`/`revert`). Pixel edits store only the dirty rectangle's before/after bytes. |
| `src/render.rs` | Texture sync (partial uploads), onion-skin cache, thumbnails. |
| `src/export/` | GIF, PNG sequence, sprite sheet, ffmpeg MP4. Every format is a step-based job so it runs on a thread natively and time-sliced on wasm. |
| `src/ui/` | Thin egui layer: theme, widgets, canvas, tool rail, timeline, dialogs, shortcuts. |
| `src/app.rs` | App state and editing operations. UI code calls these methods; it never edits pixels itself. |
| `src/platform.rs`, `src/autosave.rs` | Everything that differs between desktop and browser (dialogs, downloads, storage). |

### Ground rules

- **Keep the model pure and testable.** Anything in `model/`, `tools/`, `history.rs`, `export/` should be testable without a window. App-level behaviour is tested headlessly through `FlipLineApp::with_project`.
- **No `unwrap()` on user-facing paths.** Use `thiserror` for module errors and surface failures as toasts (`app.toasts.error(..)`). `anyhow` is for the edges only.
- **Don't upload textures every repaint.** Pixel changes must go through `Frame::touch`, which feeds the dirty rect that `render::sync_texture` uploads.
- **Everything undoable goes through `History`.** Pixel gestures return a `Command` from the tool; timeline edits use `App::run`.
- **Guard hostile input.** Project files and images are untrusted: validate sizes before allocating (see `model/io.rs`).
- **Web parity.** New features must compile for `wasm32-unknown-unknown`. Put `cfg` branches in `platform.rs`, not in UI code.

### Tests worth copying

- `export/gif_out.rs`: GIF encode, decode with the `gif` crate, compare pixels and delays.
- `tools/fill.rs`: fill on a closed shape with an anti-aliased border, plus a guard test proving the halo exists without tuck-under.
- `history.rs` and `app.rs`: undo/redo restores exact pixels across strokes, fills and timeline edits.
- `model/io.rs`: `.flipline` round trip, hostile archives.

### Performance

`cargo run --release --example perf` times the hot paths (stroke events, onion rebuild, thumbnails, upload prep, a full UI pass with 100 frames). If you touch rasterisation, caching or the timeline, run it before and after.

## Good first issues

See the roadmap in the README. Small, self-contained pieces: more onion-skin tint options, a "clear frame" shortcut, export presets, translating the help overlay.

## Licence

By contributing you agree your work is released under the MIT licence (see `LICENSE`).
