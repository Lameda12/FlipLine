//! Everything that differs between desktop and browser: file dialogs, saving/downloading,
//! autosave storage, background work. UI code talks to this module through [`Msg`]s, so it
//! never needs `cfg` branches.

use std::future::Future;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use eframe::egui::Context;
use rfd::AsyncFileDialog;

use crate::export::{Dest, Format};
use crate::model::Snapshot;

pub const PROJECT_EXT: &str = "flipline";
pub const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

pub struct PickedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Results of asynchronous platform work, drained by the app each frame.
pub enum Msg {
    ProjectOpened { name: String, bytes: Vec<u8>, path: Option<PathBuf> },
    ImagesPicked(Vec<PickedFile>),
    ImagesDecoded { frames: Vec<tiny_skia::Pixmap>, errors: Vec<String> },
    /// A project was written (or downloaded). `path` is `None` on the web.
    Saved { path: Option<PathBuf>, name: String },
    /// The user picked where an export should go.
    ExportDest(Dest),
    ExportDestCancelled,
    ExportFinished(Result<crate::export::ExportOutput, crate::export::ExportError>),
    AutosaveDone(Result<(), String>),
    FfmpegChecked(bool),
    Info(String),
    Error(String),
}

pub fn is_web() -> bool {
    cfg!(target_arch = "wasm32")
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn<F: Future<Output = ()> + Send + 'static>(f: F) {
    std::thread::spawn(move || pollster::block_on(f));
}

#[cfg(target_arch = "wasm32")]
fn spawn<F: Future<Output = ()> + 'static>(f: F) {
    wasm_bindgen_futures::spawn_local(f);
}

/// Runs CPU-heavy work off the UI thread on desktop; inline on the web (no threads there).
#[cfg(not(target_arch = "wasm32"))]
pub fn run_background(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}

#[cfg(target_arch = "wasm32")]
pub fn run_background(f: impl FnOnce() + Send + 'static) {
    f();
}

fn send(tx: &Sender<Msg>, ctx: &Context, m: Msg) {
    let _ = tx.send(m);
    ctx.request_repaint();
}

/// Reads dropped files (native: on a worker thread; web: asynchronously) and routes them:
/// `.flipline` files open as projects, everything else is treated as an image to import.
pub fn read_dropped(files: Vec<eframe::egui::DroppedFileHandle>, tx: Sender<Msg>, ctx: Context) {
    fn route(tx: &Sender<Msg>, name: String, bytes: Vec<u8>, path: Option<PathBuf>, images: &mut Vec<PickedFile>) {
        if name.to_lowercase().ends_with(".flipline") {
            let _ = tx.send(Msg::ProjectOpened { name, bytes, path });
        } else {
            images.push(PickedFile { name, bytes });
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    run_background(move || {
        let mut images = Vec::new();
        for f in files {
            let path = f.path().to_path_buf();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match f.bytes() {
                Ok(bytes) => route(&tx, name, bytes, Some(path), &mut images),
                Err(e) => {
                    let _ = tx.send(Msg::Error(format!("Couldn't read {name}: {e}")));
                }
            }
        }
        if !images.is_empty() {
            let _ = tx.send(Msg::ImagesPicked(images));
        }
        ctx.request_repaint();
    });
    #[cfg(target_arch = "wasm32")]
    spawn(async move {
        let mut images = Vec::new();
        for f in files {
            let name = f.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match f.bytes_async().await {
                Ok(bytes) => route(&tx, name, bytes, None, &mut images),
                Err(e) => {
                    let _ = tx.send(Msg::Error(format!("Couldn't read {name}: {e}")));
                }
            }
        }
        if !images.is_empty() {
            let _ = tx.send(Msg::ImagesPicked(images));
        }
        ctx.request_repaint();
    });
}

pub fn check_ffmpeg(tx: Sender<Msg>, ctx: Context) {
    run_background(move || {
        let ok = crate::export::ffmpeg_available();
        send(&tx, &ctx, Msg::FfmpegChecked(ok));
    });
}

pub fn pick_project(tx: Sender<Msg>, ctx: Context) {
    spawn(async move {
        let file = AsyncFileDialog::new()
            .add_filter("FlipLine project", &[PROJECT_EXT])
            .set_title("Open project")
            .pick_file()
            .await;
        if let Some(f) = file {
            let bytes = f.read().await;
            #[cfg(not(target_arch = "wasm32"))]
            let path = Some(f.path().to_path_buf());
            #[cfg(target_arch = "wasm32")]
            let path = None;
            send(&tx, &ctx, Msg::ProjectOpened { name: f.file_name(), bytes, path });
        }
    });
}

pub fn pick_images(tx: Sender<Msg>, ctx: Context) {
    spawn(async move {
        let files = AsyncFileDialog::new()
            .add_filter("Images", &IMAGE_EXTS)
            .set_title("Import images")
            .pick_files()
            .await;
        if let Some(files) = files {
            let mut out = Vec::with_capacity(files.len());
            for f in files {
                out.push(PickedFile { name: f.file_name(), bytes: f.read().await });
            }
            send(&tx, &ctx, Msg::ImagesPicked(out));
        }
    });
}

/// Saves the project. With `path` it writes directly; otherwise (or on the web) it asks.
pub fn save_project(snap: Snapshot, path: Option<PathBuf>, tx: Sender<Msg>, ctx: Context, cache: crate::autosave::SharedCache) {
    let encode = move |snap: Snapshot, cache: crate::autosave::SharedCache| {
        let mut guard = cache.lock().ok();
        crate::model::io::to_bytes(&snap, guard.as_deref_mut())
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        spawn(async move {
            let path = match path {
                Some(p) => p,
                None => {
                    let default = format!("{}.{PROJECT_EXT}", crate::export::sanitize_stem(&snap.name));
                    let picked = AsyncFileDialog::new()
                        .add_filter("FlipLine project", &[PROJECT_EXT])
                        .set_file_name(default)
                        .set_title("Save project")
                        .save_file()
                        .await;
                    match picked {
                        Some(f) => ensure_ext(f.path().to_path_buf(), PROJECT_EXT),
                        None => return,
                    }
                }
            };
            let name = snap.name.clone();
            match encode(snap, cache) {
                Ok(bytes) => match write_atomic(&path, &bytes) {
                    Ok(()) => send(&tx, &ctx, Msg::Saved { path: Some(path), name }),
                    Err(e) => send(&tx, &ctx, Msg::Error(format!("Could not save: {e}"))),
                },
                Err(e) => send(&tx, &ctx, Msg::Error(format!("Could not save: {e}"))),
            }
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        let name = snap.name.clone();
        match encode(snap, cache) {
            Ok(bytes) => {
                let file = format!("{}.{PROJECT_EXT}", crate::export::sanitize_stem(&name));
                match web::download(&file, &bytes, "application/zip") {
                    Ok(()) => send(&tx, &ctx, Msg::Saved { path: None, name }),
                    Err(e) => send(&tx, &ctx, Msg::Error(format!("Download failed: {e}"))),
                }
            }
            Err(e) => send(&tx, &ctx, Msg::Error(format!("Could not save: {e}"))),
        }
    }
}

pub fn ensure_ext(mut p: PathBuf, ext: &str) -> PathBuf {
    if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case(ext)) != Some(true) {
        p.set_extension(ext);
    }
    p
}

/// Writes via a temp file + rename so a crash mid-write never corrupts the previous save.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("flipline.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Asks where an export should go. On the web everything is returned as a download instead.
pub fn choose_export_dest(format: Format, stem: String, tx: Sender<Msg>, ctx: Context) {
    if is_web() {
        send(&tx, &ctx, Msg::ExportDest(Dest::Memory));
        return;
    }
    spawn(async move {
        let dest = match format {
            Format::PngSequence => AsyncFileDialog::new()
                .set_title("Choose a folder for the PNG sequence")
                .pick_folder()
                .await
                .map(|f| Dest::Dir(folder_of(&f))),
            other => {
                let ext = other.extension();
                let default = match other {
                    Format::SpriteSheet => format!("{stem}_sheet.{ext}"),
                    _ => format!("{stem}.{ext}"),
                };
                AsyncFileDialog::new()
                    .add_filter(other.label(), &[ext])
                    .set_file_name(default)
                    .set_title("Export")
                    .save_file()
                    .await
                    .map(|f| Dest::File(ensure_ext(file_of(&f), ext)))
            }
        };
        match dest {
            Some(d) => send(&tx, &ctx, Msg::ExportDest(d)),
            None => send(&tx, &ctx, Msg::ExportDestCancelled),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn file_of(f: &rfd::FileHandle) -> PathBuf {
    f.path().to_path_buf()
}
#[cfg(target_arch = "wasm32")]
fn file_of(f: &rfd::FileHandle) -> PathBuf {
    PathBuf::from(f.file_name())
}
fn folder_of(f: &rfd::FileHandle) -> PathBuf {
    file_of(f)
}

/// Hands memory-resident export output to the user (browser downloads; zipped if several files).
pub fn deliver_memory_output(files: Vec<(String, Vec<u8>)>, stem: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        if files.len() == 1 {
            let (name, bytes) = &files[0];
            return web::download(name, bytes, mime_for(name));
        }
        let zipped = zip_files(&files).map_err(|e| e.to_string())?;
        web::download(&format!("{stem}_frames.zip"), &zipped, "application/zip")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (files, stem);
        Err("memory output is only used on the web".into())
    }
}

#[cfg(target_arch = "wasm32")]
fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("gif") => "image/gif",
        Some("png") => "image/png",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
}

#[cfg(target_arch = "wasm32")]
fn zip_files(files: &[(String, Vec<u8>)]) -> zip::result::ZipResult<Vec<u8>> {
    use std::io::{Cursor, Write};
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in files {
        z.start_file(name.as_str(), opts)?;
        z.write_all(bytes)?;
    }
    Ok(z.finish()?.into_inner())
}

// ---- autosave storage ----------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
pub fn autosave_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("flipline").join("autosave.flipline"))
}

pub fn autosave_write(bytes: &[u8]) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = autosave_path().ok_or("no data directory on this platform")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        write_atomic(&path, bytes).map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::storage_set("flipline-autosave", bytes)
    }
}

pub fn autosave_read() -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read(autosave_path()?).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::storage_get("flipline-autosave")
    }
}

pub fn autosave_clear() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(p) = autosave_path() {
            let _ = std::fs::remove_file(p);
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::storage_remove("flipline-autosave");
    }
}

#[cfg(target_arch = "wasm32")]
pub mod web {
    use base64::Engine as _;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;

    fn err(e: JsValue) -> String {
        e.as_string().unwrap_or_else(|| format!("{e:?}"))
    }

    pub fn download(name: &str, bytes: &[u8], mime: &str) -> Result<(), String> {
        let window = web_sys::window().ok_or("no window")?;
        let document = window.document().ok_or("no document")?;
        let array = js_sys::Uint8Array::from(bytes);
        let parts = js_sys::Array::new();
        parts.push(&array.buffer());
        let opts = web_sys::BlobPropertyBag::new();
        opts.set_type(mime);
        let blob = web_sys::Blob::new_with_buffer_source_sequence_and_options(&parts, &opts).map_err(err)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
        let a: web_sys::HtmlAnchorElement = document
            .create_element("a")
            .map_err(err)?
            .dyn_into()
            .map_err(|_| "anchor cast failed")?;
        a.set_href(&url);
        a.set_download(name);
        a.style().set_property("display", "none").map_err(err)?;
        let body = document.body().ok_or("no body")?;
        body.append_child(&a).map_err(err)?;
        a.click();
        let _ = body.remove_child(&a);
        let _ = web_sys::Url::revoke_object_url(&url);
        Ok(())
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    pub fn storage_set(key: &str, bytes: &[u8]) -> Result<(), String> {
        let s = storage().ok_or("browser storage is unavailable")?;
        let text = base64::engine::general_purpose::STANDARD.encode(bytes);
        s.set_item(key, &text)
            .map_err(|_| "project too large for browser storage; use Save to keep it".to_string())
    }

    pub fn storage_get(key: &str) -> Option<Vec<u8>> {
        let text = storage()?.get_item(key).ok().flatten()?;
        base64::engine::general_purpose::STANDARD.decode(text).ok()
    }

    pub fn storage_remove(key: &str) {
        if let Some(s) = storage() {
            let _ = s.remove_item(key);
        }
    }
}
