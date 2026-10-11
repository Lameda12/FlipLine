//! Periodic autosave. Only frames edited since the last run are re-encoded (the PNG cache is
//! keyed by frame revision), and the work happens off the UI thread on desktop.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use eframe::egui::Context;

use crate::model::io::PngCache;
use crate::model::Snapshot;
use crate::platform::{self, Msg};

pub type SharedCache = Arc<Mutex<PngCache>>;

pub const INTERVAL_SECS: f64 = 5.0;

pub struct Autosave {
    pub cache: SharedCache,
    last_run: f64,
    /// Modification counter value that is already on disk.
    saved_gen: u64,
    in_flight: bool,
}

impl Default for Autosave {
    fn default() -> Self {
        Self {
            cache: Arc::new(Mutex::new(PngCache::default())),
            last_run: 0.0,
            saved_gen: 0,
            in_flight: false,
        }
    }
}

impl Autosave {
    pub fn mark_clean(&mut self, generation: u64) {
        self.saved_gen = generation;
    }

    pub fn finished(&mut self) {
        self.in_flight = false;
    }

    /// Kicks off an autosave if the project changed and enough time passed.
    pub fn maybe_run(&mut self, now: f64, generation: u64, busy: bool, snap: impl FnOnce() -> Snapshot, tx: &Sender<Msg>, ctx: &Context) {
        if busy || self.in_flight || generation == self.saved_gen || now - self.last_run < INTERVAL_SECS {
            return;
        }
        self.last_run = now;
        self.in_flight = true;
        self.saved_gen = generation;
        let snap = snap();
        let cache = self.cache.clone();
        let (tx, ctx) = (tx.clone(), ctx.clone());
        platform::run_background(move || {
            let result = (|| {
                let bytes = {
                    let mut guard = cache.lock().map_err(|_| "autosave cache poisoned".to_string())?;
                    crate::model::io::to_bytes(&snap, Some(&mut guard)).map_err(|e| e.to_string())?
                };
                platform::autosave_write(&bytes)
            })();
            let _ = tx.send(Msg::AutosaveDone(result));
            ctx.request_repaint();
        });
    }
}
