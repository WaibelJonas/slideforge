use std::fs::read_dir;
use std::path::{Path, PathBuf};

use crate::ExtractionOptions;
use crate::error::WsiError;
use crate::slide::Slide;
use crate::tile::Tile;

pub struct Dataset {
    paths: Vec<PathBuf>,
}

/// Which optional per-slide outputs a [`Dataset::extract`] run should
/// produce. A struct rather than bare `bool` parameters, deliberately --
/// two or more adjacent untyped bools at a call site are easy to swap by
/// accident (`dataset.extract(&opts, &root, true, false, f)`: which one's
/// which?), and this gives a natural place to add more toggles later
/// (e.g. per-slide reports) without another signature change.
#[derive(Debug, Clone, Copy, Default)]
pub struct DatasetOutputs {
    /// Save each kept tile as a loose `.jpg` under its slide's directory.
    pub tiles: bool,
    /// Write a `.tfrecord` per slide, named after the slide itself.
    pub tfrecords: bool,
    /// Write a `.pdf` tile extraction report under the output directory
    pub report: bool,
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
    ) -> Result<(), WsiError>
    where
        F: Fn(&Slide, Tile) -> Result<(), WsiError> + Sync,
    {
        let output_root = output_root.as_ref();
        std::fs::create_dir_all(output_root)?;

        for path in &self.paths {
            let slide = match Slide::open(path) {
                Ok(slide) => slide,
                Err(e) => {
                    log::error!("dataset: failed to open {}: {e}", path.display());
                    continue;
                }
            };

            let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("slide");
            let slide_root = output_root.join(file_stem);
            std::fs::create_dir_all(&slide_root)?;

            let mut slide_options = options.clone();

            // Whether each output happens at all is controlled by
            // `outputs`, not by `options.output_dir`/`tfrecord_file`
            // themselves -- those fields only mean "produce this output"
            // at the single-slide level; at dataset level that'd make
            // whether-to-produce-it depend on which fields happened to be
            // set on a shared `ExtractionOptions` template, rather than
            // being an explicit, visible choice at the call site.
            //
            // `output_dir`'s *value*, if given, is still honored as a
            // subfolder name relative to this slide's own directory under
            // `output_root` (e.g. "tiles" -> `{output_root}/{stem}/tiles/`)
            // -- a shared literal directory name across every slide is
            // fine, since each copy is already disambiguated by its own
            // `{stem}/` parent, and the files inside are coordinate-named,
            // not slide-named. With no value given, tiles go directly in
            // `{output_root}/{stem}/`.
            slide_options.output_dir = outputs.tiles.then(|| match &options.output_dir {
                Some(dir) => slide_root.join(dir),
                None => slide_root.clone(),
            });

            // `tfrecord_file`'s *value* is never used here, unlike
            // `output_dir`'s: it's a single file, not a container, so
            // every slide sharing one literal filename (e.g.
            // "tiles.tfrecord") would make the files indistinguishable
            // once they're out of their per-slide directory (flattened,
            // grepped, listed together, ...). So it's always named after
            // the slide itself instead.
            slide_options.tfrecord_file = outputs
                .tfrecords
                .then(|| slide_root.join(format!("{file_stem}.tfrecord")));

            if let Err(err) = slide.extract(&slide_options, |tile| f(&slide, tile)) {
                log::error!("dataset: extraction failed for {}: {err}", path.display());
            }
        }
        Ok(())
    }
}
