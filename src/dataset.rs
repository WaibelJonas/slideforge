use std::fs::read_dir;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::error::WsiError;
use crate::report::{DatasetReport, SlideSummary};
use crate::slide::{Slide, SlideOutputs};
use crate::tile::Tile;
use crate::{ExtractionOptions, ReportCollector};

pub struct Dataset {
    paths: Vec<PathBuf>,
}

/// Which optional per-slide outputs a [`Dataset::extract`] run should
/// produce, and where the combined dataset report goes. A struct rather
/// than bare parameters, deliberately -- gives a natural place for these
/// toggles without an ever-growing `extract` signature.
#[derive(Debug, Clone, Default)]
pub struct DatasetOutputs {
    /// Save each kept tile as a loose `.jpg` under its slide's directory.
    pub tiles: bool,
    /// Write a `.tfrecord` per slide, named after the slide itself.
    pub tfrecords: bool,
    /// Subfolder (relative to each slide's own directory under
    /// `output_root`) loose tiles are saved under, when `tiles` is set.
    /// `None` saves them directly in the slide's directory.
    pub tile_subdir: Option<PathBuf>,
    /// Where to write a combined multi-slide PDF report. `None` (default)
    /// means no report is generated.
    pub report_path: Option<PathBuf>,
}

impl Dataset {
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self, WsiError> {
        let paths = read_dir(dir)?
            .map(|result| result.map(|entry| entry.path()))
            .collect::<Result<Vec<PathBuf>, std::io::Error>>()?
            .into_iter()
            // Only `.svs` is supported so far
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("svs"))
            })
            .collect();
        Ok(Self { paths })
    }

    pub fn from_paths(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn paths(&self) -> &Vec<PathBuf> {
        &self.paths
    }

    pub fn extract<F>(
        &self,
        options: &ExtractionOptions,
        output_root: impl AsRef<Path>,
        outputs: DatasetOutputs,
        f: F,
    ) -> Result<DatasetReport, WsiError>
    where
        F: Fn(&Slide, Tile) -> Result<(), WsiError> + Sync,
    {
        let start = Instant::now();
        let output_root = output_root.as_ref();
        std::fs::create_dir_all(output_root)?;

        let mut summaries = Vec::new();
        let mut failures = Vec::new();

        for path in &self.paths {
            let slide = match Slide::open(path) {
                Ok(slide) => slide,
                Err(e) => {
                    log::error!("dataset: failed to open {}: {e}", path.display());
                    continue;
                }
            };

            let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("slide");
            //`{output_root}/{slide_root}/`
            let slide_root = output_root.join(file_stem);
            std::fs::create_dir_all(&slide_root)?;

            // if `outputs.tiles`, loose `.jpeg` tiles will be stored under
            // `{output_root}/{slide_root}/{tile_subdir}/`
            let tile_dir = outputs.tiles.then(|| match &outputs.tile_subdir {
                Some(subdir) => slide_root.join(subdir),
                None => slide_root.clone(),
            });

            // if `outputs.tfrecords`, a `.tfrecord` record will be stored at
            // `{output_root}/{slide_root}/{file_stem}.tfrecord`
            let tfrecord_file = outputs
                .tfrecords
                .then(|| slide_root.join(format!("{file_stem}.tfrecord")));

            let slide_outputs = SlideOutputs {
                tile_dir,
                tfrecord_file,
                report_path: None,
            };

            // When `outputs.report_path` is set, build a `ReportCollector`
            // and add it to the observer chain
            let collector = outputs
                .report_path
                .is_some()
                .then(|| Arc::new(ReportCollector::new()));
            let mut slide_options = options.clone();
            if let Some(c) = &collector {
                slide_options = slide_options.with_shared_observer(c.clone())
            }

            match slide.extract(&slide_options, &slide_outputs, |tile| f(&slide, tile)) {
                Ok(_) => {
                    // Only on success => capture and collect stats for later report
                    if let Some(c) = &collector {
                        let level_idx =
                            slide.resolve_resolution_level(&slide_options.extraction_level)?;
                        let level = slide
                            .metadata()
                            .level(level_idx)
                            .ok_or(WsiError::LevelIndexOutOfBounds)?;
                        let overview = slide.build_overview_image()?;

                        summaries.push(SlideSummary::new(
                            file_stem.to_string(),
                            c.kept(),
                            c.dropped(),
                            overview,
                            c.tile_positions(),
                            (level.dimensions().width, level.dimensions().height),
                            (level.tile_size().width, level.tile_size().height),
                        ));
                    }
                }
                Err(err) => {
                    log::error!("dataset: extraction failed for {}: {err}", path.display());
                    failures.push((path.clone(), err));
                }
            }
        }

        let report = DatasetReport::new(summaries, failures, options, start.elapsed());
        if let Some(path) = &outputs.report_path {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            report.write_pdf(path)?;
        }
        Ok(report)
    }
}
