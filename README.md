# Slideforge

![Whole-slide overview, and the same overview with tissue-filter-kept tiles outlined in green](docs/hero.png)

![CI](https://github.com/WaibelJonas/slideforge/actions/workflows/ci.yml/badge.svg)

[![Version](https://img.shields.io/github/v/tag/WaibelJonas/slideforge?label=version)](https://github.com/WaibelJonas/slideforge/releases)

[![License: GPL v3](https://img.shields.io/badge/License-GPL%20v3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)

[![Made with Rust](https://img.shields.io/badge/Made%20with-Rust-orange)](https://www.rust-lang.org)

---

Slideforge is a lightweight, open-source library for tile extraction on Whole Slide Images (WSI) written in Rust. It utilizes features inherent to Rust, such as memory safety and concurrency, to provide fast and efficient tile extraction. Additionally, it implements basic slide preprocessing techniques, such as tissue detection and color normalization, to enhance the quality of extracted tiles. Slideforge aims to be easily integrable into existing projects, pipelines, and workflows, and is available on Linux, macOS, and Windows.

## Features

- Aperio SVS pyramid parsing and tile extraction
- Resolution-targeted extraction: Users can pick either an exact pyramid level or a target Microns-Per-Pixel (MPP) value
- Otsu thresholding for tissue detection and filtering
- Reinhard stain normalization
- Sequential or parallel tile processing via [rayon](https://crates.io/crates/rayon)
- Tile output in JPEG format or serialized as a TensorFlow TFRecord file
- PDF extraction reports, for a single slide or an entire dataset
- Batch processing over a whole directory of slides
- Cross-platform support for Linux, macOS, and Windows

## Getting Started

This section provides an overview of how to get started with Slideforge, using either its Command Line Interface (CLI) or the Rust library.

### Installation

Slideforge isn't published on crates.io (yet ;)) but can be installed via Github.

The easiest way to get the CLI is to download a prebuilt binary from the
[Releases page](https://github.com/WaibelJonas/slideforge/releases). Windows, macOS (Intel + Apple Silicon), and Linux builds vor **v.0.1.0** are currently published.

Alternatively, you can build Slideforge yourself with cargo:

    cargo install --git https://github.com/WaibelJonas/slideforge

As a library, add the following to your `Cargo.toml`:

    [dependencies]
    slideforge = { git = "https://github.com/WaibelJonas/slideforge" }

Slideforge requires **Rust 1.85 or later**.

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

The above example extracts tiles from all slides in `./slides`, in the same manner as the single-slide example, and writes a PDF report for the entire dataset. As with `extract`, a `.tfrecord` is always written per slide, under `./output/<slide-stem>/<slide-stem>.tfrecord`; loose `.jpg` tiles are opt-in via `--tiles`.

### Library

```rust
use slideforge::slide::{Slide, SlideOutputs};
use slideforge::ExtractionOptions;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let slide = Slide::open("slide.svs")?;

    let options = ExtractionOptions::parallel()
        .with_target_mpp(0.5)
        .with_min_tissue_fraction(0.1)
        .with_stain_normalization();

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

## Documentation

Since the crate is not yet published to crates.io, the documentation is not available on docs.rs. However, you can generate the documentation locally using the following command:

    cargo doc --open --no-deps

## Resources

For more information on general concepts and techniques used in slideforge, please refer to the following resources:

- [Whole Slide Imaging (WSI)](https://en.wikipedia.org/wiki/Whole_slide_imaging)
- [Otsu's Method](https://en.wikipedia.org/wiki/Otsu%27s_method)
- [Reinhard Stain Normalization](https://ieeexplore.ieee.org/document/946629)

For more information on the Aperio SVS file format, please refer to the following resources:

- [Aperio SVS File Format](https://openslide.org/formats/aperio-svs/)
- [OpenSlide](https://openslide.org/)

The reference tile that serves as a target for stain normalization is taken from the WSI `TCGA-B9EB312E82F6` from [The Cancer Genome Atlas (TCGA)](https://www.cancer.gov/about-nci/organization/ccg/research/structural-genomics/tcga)

## License

Slideforge is licensed under the [GNU General Public License v3.0](https://www.gnu.org/licenses/gpl-3.0). see [LICENSE](LICENSE) for the full text.
