//! `.flipline` project files: a zip holding `project.json` plus `frames/0001.png`...
//!
//! Frames are stored as straight-alpha RGBA PNGs. Loading is defensive: sizes, counts and
//! PNG dimensions are validated before any large allocation, and archive entry names are only
//! ever used as lookup keys (nothing is extracted to disk).

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, Write};
use std::sync::Arc;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::frame::FrameId;
use super::pixels::{fit_contain, pixmap_from_straight, pixmap_to_straight};
use super::project::{Project, Snapshot, MAX_DIMENSION, MAX_FRAMES};
use super::timing::{MAX_HOLD, MIN_HOLD};

pub const FORMAT_NAME: &str = "flipline";
pub const FORMAT_VERSION: u32 = 1;
const MAX_JSON_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PNG_DIMENSION: u32 = 8192;

#[derive(Debug, Error)]
pub enum ProjectIoError {
    #[error("not a valid .flipline archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("project.json is malformed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("project was saved by a newer FlipLine (format v{0}, this build reads up to v{FORMAT_VERSION})")]
    TooNew(u32),
}

#[derive(Serialize, Deserialize)]
struct ProjectJson {
    format: String,
    version: u32,
    name: String,
    width: u32,
    height: u32,
    fps: u32,
    paper: String,
    frames: Vec<FrameJson>,
}

#[derive(Serialize, Deserialize)]
struct FrameJson {
    file: String,
    #[serde(default = "default_hold")]
    hold: u8,
}

fn default_hold() -> u8 {
    1
}

/// Remembers encoded PNGs by (frame id, revision) so autosave only re-encodes edited frames.
#[derive(Default)]
pub struct PngCache {
    entries: HashMap<FrameId, (u64, Arc<Vec<u8>>)>,
}

impl PngCache {
    fn retain_only(&mut self, snap: &Snapshot) {
        let live: std::collections::HashSet<_> = snap.frames.iter().map(|f| f.id).collect();
        self.entries.retain(|id, _| live.contains(id));
    }
}

pub fn encode_png_rgba(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, ProjectIoError> {
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive).write_image(
        rgba,
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn write_project<W: Write + Seek>(
    snap: &Snapshot,
    writer: W,
    mut cache: Option<&mut PngCache>,
) -> Result<(), ProjectIoError> {
    if let Some(c) = cache.as_deref_mut() {
        c.retain_only(snap);
    }
    let mut zip = ZipWriter::new(writer);
    let png_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let json_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut blank_png: Option<Arc<Vec<u8>>> = None;
    let mut frames_json = Vec::with_capacity(snap.frames.len());
    for (i, f) in snap.frames.iter().enumerate() {
        let name = format!("frames/{:04}.png", i + 1);
        let bytes: Arc<Vec<u8>> = match &f.pixmap {
            None => match &blank_png {
                Some(b) => b.clone(),
                None => {
                    let zeros = vec![0u8; snap.width as usize * snap.height as usize * 4];
                    let b = Arc::new(encode_png_rgba(snap.width, snap.height, &zeros)?);
                    blank_png = Some(b.clone());
                    b
                }
            },
            Some(pm) => {
                let hit = cache
                    .as_deref()
                    .and_then(|c| c.entries.get(&f.id))
                    .filter(|(rev, _)| *rev == f.rev)
                    .map(|(_, b)| b.clone());
                match hit {
                    Some(b) => b,
                    None => {
                        let straight = pixmap_to_straight(pm);
                        let b = Arc::new(encode_png_rgba(pm.width(), pm.height(), &straight)?);
                        if let Some(c) = cache.as_deref_mut() {
                            c.entries.insert(f.id, (f.rev, b.clone()));
                        }
                        b
                    }
                }
            }
        };
        zip.start_file(name.as_str(), png_opts)?;
        zip.write_all(&bytes)?;
        frames_json.push(FrameJson {
            file: name,
            hold: f.hold,
        });
    }

    let json = ProjectJson {
        format: FORMAT_NAME.into(),
        version: FORMAT_VERSION,
        name: snap.name.clone(),
        width: snap.width,
        height: snap.height,
        fps: snap.fps,
        paper: hex(snap.paper),
        frames: frames_json,
    };
    zip.start_file("project.json", json_opts)?;
    zip.write_all(&serde_json::to_vec_pretty(&json)?)?;
    zip.finish()?;
    Ok(())
}

pub fn to_bytes(snap: &Snapshot, cache: Option<&mut PngCache>) -> Result<Vec<u8>, ProjectIoError> {
    let mut buf = Cursor::new(Vec::new());
    write_project(snap, &mut buf, cache)?;
    Ok(buf.into_inner())
}

pub fn read_project<R: Read + Seek>(reader: R) -> Result<Project, ProjectIoError> {
    let mut zip = ZipArchive::new(reader)?;

    let json: ProjectJson = {
        let entry = zip.by_name("project.json").map_err(|_| {
            ProjectIoError::Invalid("archive has no project.json (is this a .flipline file?)".into())
        })?;
        if entry.size() > MAX_JSON_BYTES {
            return Err(ProjectIoError::Invalid("project.json is too large".into()));
        }
        let mut text = Vec::new();
        entry.take(MAX_JSON_BYTES).read_to_end(&mut text)?;
        serde_json::from_slice(&text)?
    };

    if json.format != FORMAT_NAME {
        return Err(ProjectIoError::Invalid(format!(
            "unexpected format \"{}\"",
            json.format
        )));
    }
    if json.version > FORMAT_VERSION {
        return Err(ProjectIoError::TooNew(json.version));
    }
    if json.width == 0 || json.height == 0 || json.width > MAX_DIMENSION || json.height > MAX_DIMENSION {
        return Err(ProjectIoError::Invalid(format!(
            "canvas size {}x{} is out of range (1..={MAX_DIMENSION})",
            json.width, json.height
        )));
    }
    if json.frames.is_empty() {
        return Err(ProjectIoError::Invalid("project has no frames".into()));
    }
    if json.frames.len() > MAX_FRAMES {
        return Err(ProjectIoError::Invalid(format!(
            "too many frames ({} > {MAX_FRAMES})",
            json.frames.len()
        )));
    }

    let mut project = Project::empty(json.name, json.width, json.height);
    project.fps = json.fps.clamp(1, 120);
    project.paper = parse_hex(&json.paper).unwrap_or(super::project::DEFAULT_PAPER);

    for fj in &json.frames {
        let mut bytes = Vec::new();
        {
            let entry = zip
                .by_name(&fj.file)
                .map_err(|_| ProjectIoError::Invalid(format!("missing frame file {}", fj.file)))?;
            // A PNG of the largest accepted size is far below this; the cap bounds zip bombs.
            entry.take(512 * 1024 * 1024).read_to_end(&mut bytes)?;
        }
        let pm = decode_png_bounded(&bytes, json.width, json.height)?;
        let mut frame = project.frame_from_pixmap(pm);
        frame.set_hold(fj.hold.clamp(MIN_HOLD, MAX_HOLD));
        let idx = project.len();
        project.insert_frame(idx, frame);
    }
    Ok(project)
}

pub fn from_bytes(bytes: &[u8]) -> Result<Project, ProjectIoError> {
    read_project(Cursor::new(bytes))
}

/// Decodes any supported image into a premultiplied pixmap exactly `w`x`h`, contain-fitting
/// when the source differs in size. Rejects absurd dimensions before allocating.
pub fn decode_image_fitted(bytes: &[u8], w: u32, h: u32) -> Result<tiny_skia::Pixmap, ProjectIoError> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let (iw, ih) = reader.into_dimensions()?;
    check_dims(iw, ih)?;
    let img = image::load_from_memory(bytes)?.into_rgba8();
    let src = pixmap_from_straight(img.width(), img.height(), img.as_raw())
        .ok_or_else(|| ProjectIoError::Invalid("image has zero size".into()))?;
    if src.width() == w && src.height() == h {
        Ok(src)
    } else {
        fit_contain(&src, w, h).ok_or_else(|| ProjectIoError::Invalid("bad canvas size".into()))
    }
}

fn check_dims(iw: u32, ih: u32) -> Result<(), ProjectIoError> {
    if iw == 0 || ih == 0 || iw > MAX_PNG_DIMENSION || ih > MAX_PNG_DIMENSION {
        return Err(ProjectIoError::Invalid(format!(
            "image dimensions {iw}x{ih} are out of range"
        )));
    }
    Ok(())
}

fn decode_png_bounded(bytes: &[u8], w: u32, h: u32) -> Result<tiny_skia::Pixmap, ProjectIoError> {
    let mut reader = ImageReader::new(Cursor::new(bytes));
    reader.set_format(ImageFormat::Png);
    let (iw, ih) = reader.into_dimensions()?;
    check_dims(iw, ih)?;
    decode_image_fitted(bytes, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{Color, Paint, PathBuilder, Transform};

    fn sample_project() -> Project {
        let mut p = Project::new("round trip", 96, 64);
        p.fps = 15;
        p.paper = [10, 20, 30];
        for i in 0..3u8 {
            let mut f = p.new_blank_frame();
            if i != 1 {
                // Anti-aliased, partially transparent content exercises premultiplication.
                let pm = f.pixmap_mut();
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgba8(200, 40 * i, 90, 180));
                paint.anti_alias = true;
                let path = PathBuilder::from_circle(30.5 + i as f32 * 7.0, 31.3, 17.7).unwrap();
                pm.fill_path(
                    &path,
                    &paint,
                    tiny_skia::FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
            f.set_hold(i + 1);
            let at = p.len();
            p.insert_frame(at, f);
        }
        p
    }

    #[test]
    fn save_load_roundtrip_is_pixel_exact() {
        let p = sample_project();
        let bytes = to_bytes(&p.snapshot(), None).unwrap();
        let q = from_bytes(&bytes).unwrap();

        assert_eq!(q.name, p.name);
        assert_eq!((q.width(), q.height()), (96, 64));
        assert_eq!(q.fps, 15);
        assert_eq!(q.paper, [10, 20, 30]);
        assert_eq!(q.len(), p.len());
        for (a, b) in p.frames().iter().zip(q.frames()) {
            assert_eq!(a.hold, b.hold);
            let pa = a
                .pixmap()
                .map(|p| p.data().to_vec())
                .unwrap_or_else(|| vec![0; 96 * 64 * 4]);
            let pb = b.pixmap().unwrap().data();
            assert_eq!(pa, pb, "pixels must survive PNG round trip exactly");
        }
    }

    #[test]
    fn archive_layout_matches_spec() {
        let p = sample_project();
        let bytes = to_bytes(&p.snapshot(), None).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        assert!(names.contains(&"project.json".to_string()));
        assert!(names.contains(&"frames/0001.png".to_string()));
        assert!(names.contains(&"frames/0004.png".to_string()));
    }

    #[test]
    fn png_cache_skips_unchanged_frames() {
        let p = sample_project();
        let mut cache = PngCache::default();
        let a = to_bytes(&p.snapshot(), Some(&mut cache)).unwrap();
        let before: Vec<_> = cache.entries.values().map(|(_, b)| Arc::as_ptr(b)).collect();
        let b = to_bytes(&p.snapshot(), Some(&mut cache)).unwrap();
        let after: Vec<_> = cache.entries.values().map(|(_, b)| Arc::as_ptr(b)).collect();
        assert_eq!(a.len(), b.len());
        let mut x = before;
        let mut y = after;
        x.sort();
        y.sort();
        assert_eq!(x, y, "cached encodings are reused");
    }

    #[test]
    fn rejects_garbage_and_hostile_projects() {
        assert!(from_bytes(b"not a zip").is_err());

        // Valid zip, wrong content.
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = ZipWriter::new(&mut buf);
            z.start_file("project.json", SimpleFileOptions::default())
                .unwrap();
            z.write_all(br##"{"format":"flipline","version":1,"name":"x","width":999999,"height":10,"fps":12,"paper":"#fff","frames":[{"file":"frames/0001.png","hold":1}]}"##).unwrap();
            z.finish().unwrap();
        }
        let err = from_bytes(&buf.into_inner()).err().unwrap();
        assert!(err.to_string().contains("out of range"), "{err}");

        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = ZipWriter::new(&mut buf);
            z.start_file("project.json", SimpleFileOptions::default())
                .unwrap();
            z.write_all(br##"{"format":"flipline","version":99,"name":"x","width":8,"height":8,"fps":12,"paper":"#fff","frames":[]}"##).unwrap();
            z.finish().unwrap();
        }
        assert!(matches!(
            from_bytes(&buf.into_inner()),
            Err(ProjectIoError::TooNew(99))
        ));
    }

    #[test]
    fn mismatched_frame_size_is_contain_fitted() {
        let mut big = tiny_skia::Pixmap::new(200, 100).unwrap();
        big.fill(Color::from_rgba8(255, 0, 0, 255));
        let png = encode_png_rgba(200, 100, &pixmap_to_straight(&big)).unwrap();
        let fitted = decode_image_fitted(&png, 100, 100).unwrap();
        assert_eq!((fitted.width(), fitted.height()), (100, 100));
        assert_eq!(fitted.pixel(50, 50).unwrap().alpha(), 255);
        assert_eq!(fitted.pixel(50, 2).unwrap().alpha(), 0);
    }
}
