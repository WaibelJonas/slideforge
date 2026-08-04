//! Oberserver structs with logging for tile extraction.
//!
//! The public struct [`ExtractionObserver`] constitutes logging functions
//! for all tissue-filter decisions and is to be implemented by any new Observer.
//! ['SimpleLogging'] implements basic log messages.
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// Progress bar width
const BAR_WIDTH: usize = 40;

// Summary counts for a completed extraction run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionStats {
    pub total: usize,
    pub dropped: usize,
}

impl ExtractionStats {
    // Returns the number of tiles that were kept.
    pub fn kept(&self) -> usize {
        self.total - self.dropped
    }
}

/// Receives events emitted during [`Slide::extract`](crate::slide::Slide::extract).
///
/// All methods have no-op default implementations, so implementors only
/// need to override the events they care about.
pub trait ExtractionObserver: Send + Sync {
    /// Called once before any tiles are processed, with the total number
    /// of tiles that will be considered at `level_idx`.
    fn on_extraction_start(&self, _level_idx: usize, _total_tiles: usize) {}

    /// Called when a tile is skipped because its tissue fraction fell
    /// below the configured minimum.
    fn on_tile_dropped(
        &self,
        _level_idx: usize,
        _tile_x: u32,
        _tile_y: u32,
        _tissue_fraction: f32,
        _min_fraction: f32,
    ) {
    }

    /// Called once tissue-filtered extraction finishes.
    fn on_extraction_complete(&self, _level_idx: usize, _stats: ExtractionStats) {}

    /// Called for every extracted tile
    fn on_tile_extraction(&self, _level_idx: usize, _tile_x: u32, _tile_y: u32) {}
}

/// An [`ExtractionObserver`] that reports events through the `log` crate.
///
/// Per-tile drops are logged at `debug` level, extracted tiles and the final summary at `info`
/// level.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleLogging;

impl ExtractionObserver for SimpleLogging {
    fn on_extraction_start(&self, level_idx: usize, total_tiles: usize) {
        log::info!("extract: starting extraction of {total_tiles} tiles at level {level_idx}");
    }

    fn on_tile_dropped(
        &self,
        level_idx: usize,
        tile_x: u32,
        tile_y: u32,
        tissue_fraction: f32,
        min_fraction: f32,
    ) {
        log::debug!(
            "extract: dropping tile ({tile_x}, {tile_y}) at level {level_idx}, tissue_fraction={tissue_fraction} < {min_fraction}"
        );
    }

    fn on_extraction_complete(&self, level_idx: usize, stats: ExtractionStats) {
        log::info!(
            "extract: dropped {}/{} tiles at level {level_idx} due to tissue filtering",
            stats.dropped,
            stats.total
        );
    }

    fn on_tile_extraction(&self, level_idx: usize, tile_x: u32, tile_y: u32) {
        log::info!("extract: processed tile ({tile_x}, {tile_y}) at level {level_idx}");
    }
}

/// An [`ExtractionObserver`] that renders a single-line progress bar.
#[derive(Debug, Default)]
pub struct ProgressBar {
    total: AtomicUsize,
    processed: AtomicUsize,
    dropped: AtomicUsize,
    finished: AtomicBool,
    line: Mutex<()>,
}

impl ProgressBar {
    pub fn new() -> Self {
        Self::default()
    }

    fn render(&self) {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return;
        }

        let processed = self.processed.load(Ordering::Relaxed).min(total);
        let dropped = self.dropped.load(Ordering::Relaxed);

        let filled = processed * BAR_WIDTH / total;
        let bar = "#".repeat(filled) + &"-".repeat(BAR_WIDTH - filled);
        let percent = processed * 100 / total;

        let _guard = self.line.lock().unwrap();
        eprint!("\r[{bar}] {percent:>3}% ({processed}/{total} tiles, {dropped} dropped)");

        if processed >= total && !self.finished.swap(true, Ordering::Relaxed) {
            eprintln!();
        }

        let _ = std::io::stderr().flush();
    }
}

impl ExtractionObserver for ProgressBar {
    fn on_extraction_start(&self, _level_idx: usize, total_tiles: usize) {
        self.total.store(total_tiles, Ordering::Relaxed);
        self.processed.store(0, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
        self.finished.store(false, Ordering::Relaxed);
        self.render();
    }

    fn on_tile_dropped(
        &self,
        _level_idx: usize,
        _tile_x: u32,
        _tile_y: u32,
        _tissue_fraction: f32,
        _min_fraction: f32,
    ) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        self.processed.fetch_add(1, Ordering::Relaxed);
        self.render();
    }

    fn on_tile_extraction(&self, _level_idx: usize, _tile_x: u32, _tile_y: u32) {
        self.processed.fetch_add(1, Ordering::Relaxed);
        self.render();
    }
}

/// An [`ExtractionObserver`] that collects summary counts, per-tile
/// keep/drop positions and dropped-tile tissue fractions for building an
/// extraction report.
#[derive(Debug, Default)]
pub(crate) struct ReportCollector {
    total: AtomicUsize,
    kept: AtomicUsize,
    dropped: AtomicUsize,
    // (tile_x, tile_y, kept)
    positions: Mutex<Vec<(u32, u32, bool)>>,
    dropped_tissue_fractions: Mutex<Vec<f32>>,
}

impl ReportCollector {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    pub(crate) fn kept(&self) -> usize {
        self.kept.load(Ordering::Relaxed)
    }

    pub(crate) fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    pub(crate) fn tile_positions(&self) -> Vec<(u32, u32, bool)> {
        self.positions.lock().expect("Mutex lock failed!").clone()
    }

    pub(crate) fn dropped_tissue_fractions(&self) -> Vec<f32> {
        self.dropped_tissue_fractions
            .lock()
            .expect("Mutex lock failed!")
            .clone()
    }
}

impl ExtractionObserver for ReportCollector {
    fn on_extraction_start(&self, _level_idx: usize, total_tiles: usize) {
        self.total.store(total_tiles, Ordering::Relaxed);
    }

    fn on_tile_dropped(
        &self,
        _level_idx: usize,
        tile_x: u32,
        tile_y: u32,
        tissue_fraction: f32,
        _min_fraction: f32,
    ) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        self.positions
            .lock()
            .expect("Mutex lock failed!")
            .push((tile_x, tile_y, false));
        self.dropped_tissue_fractions
            .lock()
            .expect("Mutex lock failed!")
            .push(tissue_fraction);
    }

    fn on_tile_extraction(&self, _level_idx: usize, tile_x: u32, tile_y: u32) {
        self.kept.fetch_add(1, Ordering::Relaxed);
        self.positions
            .lock()
            .expect("Mutex lock failed!")
            .push((tile_x, tile_y, true));
    }
}

/// An [`ExtractionObserver`] that forwards every event to two other
/// observers.
/// Designed for use with a user-supplied observer such as [`SimpleLogging`]
/// and an underlying observer such as [`ReportCollector`]
pub(crate) struct DualObserver(
    pub(crate) Arc<dyn ExtractionObserver>,
    pub(crate) Arc<dyn ExtractionObserver>,
);

impl ExtractionObserver for DualObserver {
    fn on_extraction_start(&self, level_idx: usize, total_tiles: usize) {
        self.0.on_extraction_start(level_idx, total_tiles);
        self.1.on_extraction_start(level_idx, total_tiles);
    }

    fn on_tile_dropped(
        &self,
        level_idx: usize,
        tile_x: u32,
        tile_y: u32,
        tissue_fraction: f32,
        min_fraction: f32,
    ) {
        self.0
            .on_tile_dropped(level_idx, tile_x, tile_y, tissue_fraction, min_fraction);
        self.1
            .on_tile_dropped(level_idx, tile_x, tile_y, tissue_fraction, min_fraction);
    }

    fn on_extraction_complete(&self, level_idx: usize, stats: ExtractionStats) {
        self.0.on_extraction_complete(level_idx, stats);
        self.1.on_extraction_complete(level_idx, stats);
    }

    fn on_tile_extraction(&self, level_idx: usize, tile_x: u32, tile_y: u32) {
        self.0.on_tile_extraction(level_idx, tile_x, tile_y);
        self.1.on_tile_extraction(level_idx, tile_x, tile_y);
    }
}
