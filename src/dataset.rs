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
        let mut succeeded = 0usize;
        let mut total_kept = 0usize;
        let mut total_dropped = 0usize;

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

            // Cheap; always attached so kept/dropped stay accurate without a report.
            let collector = Arc::new(ReportCollector::new());
            let slide_options = options.clone().with_shared_observer(collector.clone());

            match slide.extract(&slide_options, &slide_outputs, |tile| f(&slide, tile)) {
                Ok(_) => {
                    succeeded += 1;
                    total_kept += collector.kept();
                    total_dropped += collector.dropped();

                    // Overview stitching is expensive; only do it if a report was requested.
                    if outputs.report_path.is_some() {
                        let level_idx =
                            slide.resolve_resolution_level(&slide_options.extraction_level)?;
                        let level = slide
                            .metadata()
                            .level(level_idx)
                            .ok_or(WsiError::LevelIndexOutOfBounds)?;
                        let overview = slide.build_overview_image()?;

                        summaries.push(SlideSummary::new(
                            file_stem.to_string(),
                            collector.kept(),
                            collector.dropped(),
                            overview,
                            collector.tile_positions(),
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

        let report = DatasetReport::new(
            summaries,
            failures,
            options,
            start.elapsed(),
            succeeded,
            total_kept,
            total_dropped,
        );
        if let Some(path) = &outputs.report_path {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            report.write_pdf(path)?;
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// See `tests/fixtures/README.md` for provenance.
    const FIXTURE: &str = "tests/fixtures/CMU-1-Small-Region.svs";

    /// Copies the fixture into `dir` `count` times, as `slide_N.svs`.
    fn populate_with_fixture_copies(dir: &Path, count: usize) {
        for i in 0..count {
            fs::copy(FIXTURE, dir.join(format!("slide_{i}.svs"))).unwrap();
        }
    }

    #[test]
    fn from_dir_picks_up_only_svs_files_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        fs::copy(FIXTURE, dir.path().join("a.svs")).unwrap();
        fs::copy(FIXTURE, dir.path().join("b.SVS")).unwrap();
        fs::write(dir.path().join("notes.txt"), b"not a slide").unwrap();

        let dataset = Dataset::from_dir(dir.path()).unwrap();
        assert_eq!(dataset.len(), 2);
    }

    #[test]
    fn from_dir_on_empty_directory_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let dataset = Dataset::from_dir(dir.path()).unwrap();
        assert!(dataset.is_empty());
        assert_eq!(dataset.len(), 0);
    }

    #[test]
    fn from_paths_uses_the_given_paths_directly() {
        let dataset = Dataset::from_paths(vec![PathBuf::from("a.svs"), PathBuf::from("b.svs")]);
        assert_eq!(dataset.len(), 2);
        assert_eq!(dataset.paths().len(), 2);
    }

    #[test]
    fn extract_processes_every_slide_and_invokes_the_callback_per_kept_tile() {
        let input_dir = tempfile::tempdir().unwrap();
        populate_with_fixture_copies(input_dir.path(), 2);
        let dataset = Dataset::from_dir(input_dir.path()).unwrap();
        assert_eq!(dataset.len(), 2);

        let output_dir = tempfile::tempdir().unwrap();
        let options = ExtractionOptions::sequential().with_level(0);

        let callback_calls = AtomicUsize::new(0);
        let report = dataset
            .extract(
                &options,
                output_dir.path(),
                DatasetOutputs::default(),
                |_slide, _tile| {
                    callback_calls.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(report.total_slides(), 2);
        assert_eq!(report.succeeded(), 2);

        let tiles_per_slide = Slide::open(FIXTURE)
            .unwrap()
            .tile_coords(0)
            .unwrap()
            .count();
        assert_eq!(callback_calls.load(Ordering::Relaxed), tiles_per_slide * 2);
    }

    #[test]
    fn extract_writes_per_slide_tiles_and_tfrecords_under_their_own_directories() {
        let input_dir = tempfile::tempdir().unwrap();
        populate_with_fixture_copies(input_dir.path(), 1);
        let dataset = Dataset::from_dir(input_dir.path()).unwrap();

        let output_dir = tempfile::tempdir().unwrap();
        let options = ExtractionOptions::sequential().with_level(0);
        let outputs = DatasetOutputs {
            tiles: true,
            tfrecords: true,
            ..Default::default()
        };

        dataset
            .extract(&options, output_dir.path(), outputs, |_slide, _tile| Ok(()))
            .unwrap();

        let slide_dir = output_dir.path().join("slide_0");
        assert!(slide_dir.join("slide_0.tfrecord").is_file());

        let expected_tiles = Slide::open(FIXTURE)
            .unwrap()
            .tile_coords(0)
            .unwrap()
            .count();
        let written_tiles = fs::read_dir(&slide_dir)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "jpg")
            })
            .count();
        assert_eq!(written_tiles, expected_tiles);
    }

    #[test]
    fn extract_respects_tile_subdir() {
        let input_dir = tempfile::tempdir().unwrap();
        populate_with_fixture_copies(input_dir.path(), 1);
        let dataset = Dataset::from_dir(input_dir.path()).unwrap();

        let output_dir = tempfile::tempdir().unwrap();
        let options = ExtractionOptions::sequential().with_level(0);
        let outputs = DatasetOutputs {
            tiles: true,
            tile_subdir: Some(PathBuf::from("tiles")),
            ..Default::default()
        };

        dataset
            .extract(&options, output_dir.path(), outputs, |_slide, _tile| Ok(()))
            .unwrap();

        let tile_subdir = output_dir.path().join("slide_0").join("tiles");
        assert!(tile_subdir.is_dir());
        assert!(fs::read_dir(&tile_subdir).unwrap().count() > 0);
    }

    #[test]
    fn extract_skips_unopenable_slides_without_aborting_the_batch() {
        let input_dir = tempfile::tempdir().unwrap();
        populate_with_fixture_copies(input_dir.path(), 1);
        fs::write(input_dir.path().join("corrupt.svs"), b"not a real tiff").unwrap();

        let dataset = Dataset::from_dir(input_dir.path()).unwrap();
        assert_eq!(dataset.len(), 2, "both files are picked up by extension");

        let output_dir = tempfile::tempdir().unwrap();
        let options = ExtractionOptions::sequential().with_level(0);

        let report = dataset
            .extract(
                &options,
                output_dir.path(),
                DatasetOutputs::default(),
                |_slide, _tile| Ok(()),
            )
            .unwrap();

        // Open failures are skipped entirely, not counted as failures.
        assert_eq!(report.succeeded(), 1);
        assert_eq!(report.total_slides(), 1);
    }

    #[test]
    fn extract_writes_a_combined_report_when_report_path_is_set() {
        let input_dir = tempfile::tempdir().unwrap();
        populate_with_fixture_copies(input_dir.path(), 2);
        let dataset = Dataset::from_dir(input_dir.path()).unwrap();

        let output_dir = tempfile::tempdir().unwrap();
        let report_path = output_dir.path().join("combined.pdf");
        let options = ExtractionOptions::sequential().with_level(0);
        let outputs = DatasetOutputs {
            report_path: Some(report_path.clone()),
            ..Default::default()
        };

        dataset
            .extract(&options, output_dir.path(), outputs, |_slide, _tile| Ok(()))
            .unwrap();

        let bytes = fs::read(&report_path).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
    }
}
