# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-25

### 🚀 Features

- Added initial Rust project scaffold
- Added basic dependencies
- Added metadata module
- Added Aperio SVS Reader. Some changes to Metadata.
- Added base slide.rs implementation
- Added tile.rs module and basic tile reading functionality
- Added first decoder module scaffold
- Added JPEG decoder and photometric interpretation handling
- Added Tile struct and first extraction functions
- Added parallel and sequential options for tile extraction. Various additional documentation
- Added Otsu's thresholding and tile extraction logging
- Added cropping of overpadded tiles to the precprocessing steps
- Added Reinhard stain normalization as a preprocessing method
- Added Observers for the later addition of a tile extraction report
- Added Slide-level extraction reports
- Removed additional extraction methods and rolled them into slide.extract
- Added a Tfrecord writer
- Added target microns per pixel as extraction option
- Added resizing functionality to extraction pipeline
- Added dataset-level tile extraction
- Added a (rudimentary) dataset-level .pdf report. Moved slide-level file destination specifiers to dedicated SlideOutputs struct
- Added closure usage example
- Added a small clap CLI

### 🐛 Bug Fixes

- Rolled slide report creation into a builder method for ExtractionOptions
- Small batch of fixes via cargo clippy --fix
- Added bundled font and ran cargo fmt
- Added missing create-release job and dry-run flag to release workflow
- Fixed path to overview image

### 📚 Documentation

- Updated README
- Added overview image to README

### ⚙️ Miscellaneous Tasks

- Added workflow for cargo fmt/clippy/test
- Added a cross-platform release workflow
