//! Oberserver structs with logging for tile extraction.
//!
//! The public struct [`ExtractionObserver`] constitutes logging functions
//! for all tissue-filter decisions and is to be implemented by any new Observer.
//! ['SimpleLogging'] implements basic log messages-

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
