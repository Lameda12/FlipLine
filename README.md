# FlipLine

A free, fast, MIT-licensed frame-by-frame 2D animation editor. Draw a frame, flip, onion skin, time it, export. Native on macOS, Windows and Linux, and in the browser.

<!-- Replace with a real capture: record a 10 s GIF of drawing a few frames, flipping with `,` `.`, and pressing Space to play. -->
![FlipLine screenshot placeholder](docs/screenshot.gif)

[Try it in the browser](https://lameda12.github.io/FlipLine/) (enable GitHub Pages with source "GitHub Actions" for your fork) · [Download](https://github.com/Lameda12/FlipLine/releases)

## Why it exists

In February 2026 Adobe announced it was discontinuing Animate (the app formerly known as Flash). Animators publicly asked for it to be open-sourced, and Adobe backed off into maintenance mode: no new features. FlipLine is a small, open replacement for the part of Animate that people actually animate in: the drawing, flipping and timing loop. It does not try to be a scripting platform or a vector authoring suite.

## Features (v0.1)

- **Canvas**: fixed project size with presets (960×540, 1280×720, 1080×1080, 720×1280, 640×480), paper colour, a decorative peg bar above the paper. Fit to window, zoom (Ctrl/Cmd + wheel or pinch), pan (Space + drag or middle mouse).
- **Tools**: round brush with smoothing and pressure-variable width, eraser, scanline flood fill with tolerance that tucks under anti-aliased edges (no halo), colour picker. Eight ink swatches plus a custom colour. Size 1 to 60.
- **Frames**: every drawing is its own transparent RGBA layer. Add blank, duplicate, delete, reorder (drag in the timeline or Alt + arrows).
- **Holds**: each frame holds 1 to 8 exposures (animate on 1s, 2s, 3s). Timeline cells grow with the hold and show the held span.
- **Onion skin**: depth 1 to 3, past frames tinted red, future green, fading with distance. The composite is cached and rebuilt only when it changes.
- **Timeline**: thumbnails, frame numbers, active highlight, timecode (`frame 03/12 · 0.17s of 1.00s`), auto-scroll to the active frame.
- **Playback**: 6, 8, 12, 15, 24, 30 fps, loop toggle, holds respected, onion hidden while playing. Timing comes from elapsed time, not repaint count.
- **Undo/redo**: one history for strokes, fills and timeline edits. Pixel edits store only the dirty rectangle's before/after bytes. 50 entries (plus a 192 MB budget for huge fills).
- **Export**: animated GIF (per-frame 256-colour palette, infinite loop, `hold/fps` delays), PNG sequence, transparent sprite sheet (square-ish grid), MP4 through `ffmpeg`. 100% or 50% scale, progress bar, encoding off the UI thread.
- **Files**: save/open `.flipline`, import images (multi-select or drop a numbered sequence; natural sort, contain-fit), autosave every few seconds when dirty.
- **Sample** on first launch: a 12-frame bouncing ball with squash on contact, stretch near the ground and a height-scaled shadow, generated in code.
- **Light and dark themes** follow the OS.

## Install

**Download** a build for your OS from [Releases](https://github.com/Lameda12/FlipLine/releases) (macOS arm64/x64, Windows x64, Linux x64), unzip and run `flipline`. The macOS and Windows builds are unsigned: expect a Gatekeeper / SmartScreen prompt the first time.

**From source** (Rust 1.95 or newer, no system libraries needed to build):

```sh
cargo install --git https://github.com/Lameda12/FlipLine
flipline                      # or: flipline my-project.flipline
```

MP4 export needs [`ffmpeg`](https://ffmpeg.org/download.html) on your `PATH`. The option is hidden when ffmpeg is missing and in the web build.

On Linux the file dialogs use the XDG desktop portal (`xdg-desktop-portal` plus a backend such as `-gtk` or `-kde`, present on mainstream desktops). If no portal is running, Open/Save/Export will appear to do nothing.

## Build

### Native

```sh
git clone https://github.com/Lameda12/FlipLine && cd FlipLine
cargo run --release
```

### Web (wasm32 via trunk)

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk      # or grab a release from https://github.com/trunk-rs/trunk
trunk serve --release             # http://127.0.0.1:8080
trunk build --release             # static site in ./dist
```

The browser build runs the same code. Differences: saving and exports arrive as downloads (a PNG sequence is zipped), there is no MP4, encoding runs in small time slices on the UI thread instead of a worker thread, and autosave goes to `localStorage` (about 5 MB, so large projects need an explicit Save).

### Tests and checks

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown --lib --bins -- -D warnings
cargo fmt --all -- --check
cargo run --release --example perf     # timings for the hot paths
```

## Shortcuts

| Key | Action |
| --- | --- |
| `B` `E` `G` `I` | Brush, eraser, fill, colour picker (the picker returns to your previous tool after one pick) |
| `[` `]` | Smaller / larger brush |
| `,` `.` or `←` `→` | Previous / next frame |
| `Alt` + `←` `→` | Move the frame earlier / later |
| `N` | New blank frame after the current one |
| `D` | Duplicate frame |
| `Delete` | Delete frame (the last frame is cleared instead) |
| `-` `=` | Shorter / longer hold |
| `O` | Toggle onion skin |
| `Space` | Play / pause. Hold Space and drag to pan |
| `Ctrl/Cmd` + wheel, pinch | Zoom |
| `F` / `1` | Fit to window / 100% |
| `Ctrl/Cmd` + `Z` | Undo |
| `Shift` + `Ctrl/Cmd` + `Z` (or `Ctrl/Cmd` + `Y`) | Redo |
| `Ctrl/Cmd` + `S` / `Shift` + `Ctrl/Cmd` + `S` | Save / Save as |
| `Ctrl/Cmd` + `E` | Export |
| `?` or `F1` | Shortcut overlay |

## Pen pressure support

FlipLine uses pen or touch force when the platform provides it and falls back to constant width otherwise. What the platform provides was checked by reading the source of the exact versions in `Cargo.lock` (egui-winit 0.36.2, winit 0.30.13, eframe 0.36.2). It has **not** been tested on pen hardware yet; please report results.

| Platform | Pressure | Why |
| --- | --- | --- |
| Windows (Wacom, Surface Pen, other WM_POINTER pens) | Yes, expected | winit reads `pressure` from `GetPointerPenInfo` (range 1 to 1024) and emits `Touch { force }`, which egui forwards. Touchscreens use the same path. |
| macOS (Wacom and other tablets) | No, constant width | winit 0.30 does not forward tablet pressure. It only reports `TouchpadPressure` (Force Touch trackpads), which egui-winit ignores. |
| Linux X11 | No, constant width | winit emits touch events from XInput2 with `force: None // TODO`. Tablet pressure is not exposed. |
| Linux Wayland | No, constant width | winit touch events carry `force: None`; the tablet protocol is not used. |
| Web, touch and styluses that send touch events (iPad with Apple Pencil, many Android styluses) | Yes, expected | eframe forwards `Touch.force()` from `touchstart/move/end`. Devices that report 0 are treated as "no pressure". |
| Web, desktop pens (Chrome/Edge/Firefox with a tablet) | Yes, expected | eframe 0.36 does not forward pointer-event pressure, so FlipLine adds its own `pointerdown/move` listener for `pointerType == "pen"` and reads `PointerEvent.pressure`. Verified with synthetic pen events in headless Chromium, not with a real pen. |

The rail shows "Pen pressure: on" once a pressure value has been seen. Width maps linearly from 12% (zero pressure) to 100% (full) of the chosen size.

Adding macOS/Linux tablet pressure means going around winit (an `NSEvent` hook on macOS, XInput2 valuators on X11). It is on the roadmap but not in v0.1.

## File format (`.flipline`)

A `.flipline` file is a ZIP archive:

```
project.json
frames/0001.png
frames/0002.png
...
```

`project.json`:

```json
{
  "format": "flipline",
  "version": 1,
  "name": "Bouncing ball",
  "width": 960,
  "height": 540,
  "fps": 12,
  "paper": "#faf7ee",
  "frames": [
    { "file": "frames/0001.png", "hold": 1 },
    { "file": "frames/0002.png", "hold": 3 }
  ]
}
```

- `frames` is in timeline order. `hold` is the number of exposures (1 to 8, default 1). Total exposures at `fps` give the duration.
- Frame images are straight-alpha RGBA8 PNGs of exactly `width`×`height`. A different size is contain-fitted on load. Blank frames are stored as fully transparent PNGs.
- Readers must reject `version` greater than they support. Writers must not reuse a version number for incompatible changes.
- Limits enforced on load: canvas up to 4096 px per side, at most 999 frames, PNG dimensions up to 8192 px, `project.json` up to 8 MB. Archive entry names are only used as lookup keys, nothing is extracted to disk.

Because it is just a zip of PNGs, you can unzip a project to recover frames with any tool.

## Export notes

- **GIF**: each frame gets its own palette: exact when the frame has 256 colours or fewer (flat-shaded animation usually does), NeuQuant otherwise. Loops forever. Delays are `hold/fps` in centiseconds with the rounding error carried forward, so 24 fps stays 1.00 s per 24 frames instead of drifting. GIF has 1-bit transparency: the transparent option thresholds alpha at 50%. Many browsers clamp delays under 2 cs, which does not affect 30 fps or slower.
- **PNG sequence**: `name_0001.png`... One image per exposure by default so playback at the project fps keeps the timing; untick to get one per drawing. Web builds download a `.zip`.
- **Sprite sheet**: transparent PNG, one cell per drawing, `ceil(sqrt(n))` columns. Refused above 16384 px per side or about 640 MB of pixels with a hint to use 50%.
- **MP4**: raw RGBA frames piped to `ffmpeg` (`libx264`, `yuv420p`, CRF 17), holds expanded so timing is exact, drawn over the paper colour. If your ffmpeg lacks `libx264` the error from ffmpeg is shown.

## Performance

`cargo run --release --example perf` on a 1280×720 project with 100 fully drawn frames. Numbers below are from an x86-64 Linux container, **not** an M2 MacBook Air (none was available); treat them as a relative guide.

| Operation | Time |
| --- | --- |
| One pointer event while brushing (24 px brush) | 0.11 ms (about 1.8 ms per 60 fps frame at 1000 Hz input) |
| Start of a stroke (copy-on-write of the frame) | 0.33 ms |
| End of a stroke (undo entry from the dirty rect) | about 1.9 ms for a long stroke |
| Flood fill, large region on a busy frame | 13 ms |
| Onion composite rebuild (depth 2, four layers) | 5.7 ms, only when frame, depth, toggle or a neighbour changes |
| Thumbnail for one frame | 0.8 ms |
| Whole egui pass plus tessellation with 100 timeline cells | 0.5 to 1.3 ms |
| Partial texture upload prep for a typical stroke rectangle | 0.006 ms |

100 fully drawn 1280×720 frames occupy about 370 MB of pixel memory; blank frames cost nothing and duplicates share pixels until edited. GPU upload and drawing are not in these numbers. What keeps drawing smooth: textures are uploaded only when a frame is dirty, strokes upload only their dirty rectangle, and non-neighbouring frames' textures are freed.

## Architecture

See [CONTRIBUTING.md](CONTRIBUTING.md) for the module map and ground rules. In short: `model/` is pure data, `tools/` mutate a frame and return undo commands, `history.rs` is the command stack, `render.rs` owns GPU caches, `export/` is step-based so one implementation runs on a thread or time-sliced on wasm, and `ui/` is a thin egui layer.

## Known limitations

- Pen pressure is unavailable on macOS and Linux (see table) and untested on real hardware.
- One drawing layer per frame, bitmap only. No layers, selection, lasso or transform tools yet.
- Memory grows with drawn frames (about 3.7 MB each at 720p). There is no on-disk spill or in-memory compression.
- Web autosave uses `localStorage` (about 5 MB). Large projects need an explicit Save. Web exports run on the UI thread in slices.
- Fill works on the current frame's pixels only and does not close gaps in outlines.
- Undo history is capped at 50 entries and is not saved with the file.
- The macOS and Windows release binaries are unsigned.

## Roadmap

- Layers per frame
- Lasso select and transform (move, scale, rotate)
- Audio track for lip sync (waveform scrubbing)
- Vector strokes (resolution-independent lines you can edit after drawing)
- Light-table reference frames (pin any frame, or an imported image, as a faint guide)
- Tablet pressure on macOS and Linux
- Gap-closing fill, palette management, frame-range loop

## Licence

MIT. Copyright 2026 Alamedin Sabit. See [LICENSE](LICENSE).
