# Slideforge

![Whole-slide overview, and the same overview with tissue-filter-kept tiles outlined in green](docs/hero.png)

![CI](https://github.com/WaibelJonas/slideforge/actions/workflows/ci.yml/badge.svg)

[![crates.io](https://img.shields.io/crates/v/slideforge.svg)](https://crates.io/crates/slideforge)

[![docs.rs](https://img.shields.io/docsrs/slideforge)](https://docs.rs/slideforge)

[![License: GPL v3](https://img.shields.io/badge/License-GPL%20v3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)

[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org)

---

Slideforge is a lightweight, open-source library for tile extraction on Whole Slide Images (WSI) written in Rust. It utilizes features inherent to Rust, such as memory safety and concurrency, to provide fast and efficient tile extraction. Additionally, it implements basic slide preprocessing techniques, such as tissue detection and color normalization, to enhance the quality of extracted tiles. Slideforge aims to be easily integrable into existing projects, pipelines, and workflows, and is available on Linux, macOS, and Windows, as a CLI, a Rust library, and a Python package.

## Features

- Aperio SVS pyramid parsing and tile extraction
- Resolution-targeted extraction: Users can pick either an exact pyramid level or a target Microns-Per-Pixel (MPP) value
- Two tile geometries: the slide's native tile grid, or fixed-size windows on a regular grid defined in microns (grid mode)
- Otsu thresholding for tissue detection and filtering. Background is dropped on the slide's low-resolution overview before any full-resolution data is read
- Reinhard stain normalization
- Sequential or parallel tile processing via [rayon](https://crates.io/crates/rayon)
- Tile output in JPEG format or serialized as a TensorFlow TFRecord file, with configurable JPEG quality and chroma subsampling
- PDF extraction reports, for a single slide or an entire dataset
- Batch processing over a whole directory of slides
- Python bindings for downstream ML pipelines (see [python/README.md](python/README.md))
- Cross-platform support for Linux, macOS, and Windows

## Planned

- Additional tissue filtering and stain normalization methods beyond Otsu thresholding and Reinhard normalization
- Unit test coverage for `cli.rs`, `report.rs`, `tfrecord.rs`, `logging/`, and `decoder/`
- Prebuilt Python wheels on PyPI

## Getting Started

This section provides an overview of how to get started with Slideforge, using either its Command Line Interface (CLI), the Rust library, or the Python bindings.

### Installation

Slideforge is [published on crates.io](https://crates.io/crates/slideforge). To get the CLI:

    cargo install slideforge

As a library, add it to your `Cargo.toml`:

    [dependencies]
    slideforge = "0.4"

Alternatively, download a prebuilt CLI binary from the
[Releases page](https://github.com/WaibelJonas/slideforge/releases) — Windows, macOS (Intel + Apple Silicon), and Linux builds are published with every release. Building from the git repository directly (`cargo install --git https://github.com/WaibelJonas/slideforge`) also works, if you want the latest unreleased commit.

Slideforge requires **Rust 1.85 or later**.

The Python bindings are built from source with [maturin](https://www.maturin.rs/); see [python/README.md](python/README.md).

### CLI

    slideforge extract slide.svs \
      --target-mpp 0.5 \
      --min-tissue-fraction 0.1 \
      --stain-normalize \
      --tile-dir tiles/ \
      --report report.pdf

The above example extracts tiles from `slide.svs`, resampled to 0.5 microns-per-pixel, drops tiles that are
mostly background, applies Reinhard stain normalization, saves loose `.jpg`
tiles to `tiles/`, and writes a PDF extraction report. A `.tfrecord` is
always written alongside (default: `<slide-stem>.tfrecord`, override with
`--tfrecord-file`).

    slideforge dataset ./slides ./output --target-mpp 0.5 --min-tissue-fraction 0.1 --report dataset_report.pdf

The above example extracts tiles from all slides in `./slides`, in the same manner as the single-slide example, and writes a PDF report for the entire dataset. As with `extract`, a `.tfrecord` is always written per slide, under `./output/<slide-stem>/<slide-stem>.tfrecord`, loose `.jpg` tiles are opt-in via `--tiles`.

#### Tissue filtering

`--min-tissue-fraction` keeps only tiles whose share of tissue pixels is at least the given fraction. A single Otsu threshold is derived from the slide's lowest-resolution pyramid level and shared by every tile, so a tile of uniform background is not split into a false tissue/background pair. Filtering runs in two stages: tiles that are background on that low-resolution overview are dropped before their full-resolution data is read or decoded, and the remaining tiles are checked again at full resolution.

#### JPEG encoding

Tiles written to `.jpg` and `.tfrecord` are encoded at JPEG quality 75 with 4:2:0 chroma subsampling by default. Both can be changed:

    slideforge extract slide.svs --target-mpp 0.5 --jpeg-quality 95 --chroma-subsampling 4:4:4

`--chroma-subsampling` accepts `4:4:4` (full color resolution), `4:2:2` (color halved horizontally) or `4:2:0` (color halved in both directions); `444`, `422` and `420` work as well. 4:2:0 matches what libjpeg and Pillow, and therefore tools such as slideflow, produce at any quality. 4:4:4 keeps more color detail at the cost of larger files (roughly a third larger at quality 95). Grayscale tiles ignore this setting.

### Library

```rust
use slideforge::slide::{Slide, SlideOutputs};
use slideforge::{ChromaSubsampling, ExtractionOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let slide = Slide::open("slide.svs")?;

    let options = ExtractionOptions::parallel()
        .with_target_mpp(0.5)
        .with_min_tissue_fraction(0.1)
        .with_stain_normalization()
        .with_jpeg_quality(95)
        .with_chroma_subsampling(ChromaSubsampling::Cs420);

    // Governs output channels
    let outputs = SlideOutputs::default().with_tile_dir("tiles/");

    // Closure defines additional behaviour
    slide.extract(&options, &outputs, |tile| {
        println!("kept tile ({}, {})", tile.tile_x(), tile.tile_y());
        Ok(())
    })?;

    Ok(())
}
```

#### Grid mode

By default, tiles follow the slide's native tile grid. `with_grid_geometry` switches to fixed-size windows on a regular grid defined in physical units, independent of how the slide is stored:

```rust
// 256 px tiles, each covering 128 µm, with window origins 128 µm apart
let options = ExtractionOptions::parallel()
    .with_target_mpp(0.5)
    .with_grid_geometry(256, 128.0, None);
```

Each window is read at the selected level and resized to `tile_px`. Passing `Some(stride_um)` sets the distance between window origins, for overlapping or spaced-out windows; `None` places them edge to edge. The window size in pixels is truncated to whole pixels, matching slideflow's tiling convention. Unlike native boundary tiles, a window that would extend past the slide's edge is dropped rather than padded. Grid mode is available in the library and the Python bindings, but not yet in the CLI.

## Documentation

Full API documentation is on [docs.rs/slideforge](https://docs.rs/slideforge). To generate it locally instead:

    cargo doc --open --no-deps

## Resources

For more information on general concepts and techniques used in slideforge, please refer to the following resources:

- [Whole Slide Imaging (WSI)](https://en.wikipedia.org/wiki/Whole_slide_imaging)
- [Otsu's Method](https://en.wikipedia.org/wiki/Otsu%27s_method)
- [Reinhard Stain Normalization](https://ieeexplore.ieee.org/document/946629)
- [Chroma Subsampling](https://en.wikipedia.org/wiki/Chroma_subsampling)

For more information on the Aperio SVS file format, please refer to the following resources:

- [Aperio SVS File Format](https://openslide.org/formats/aperio-svs/)
- [OpenSlide](https://openslide.org/)

The reference tile that serves as a target for stain normalization is taken from the WSI `TCGA-B9EB312E82F6` from [The Cancer Genome Atlas (TCGA)](https://www.cancer.gov/about-nci/organization/ccg/research/structural-genomics/tcga)

## License

Slideforge is licensed under the [GNU General Public License v3.0](https://www.gnu.org/licenses/gpl-3.0). see [LICENSE](LICENSE) for the full text.

See [CHANGELOG.md](CHANGELOG.md) for release history.
