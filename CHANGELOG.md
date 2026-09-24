# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-09-24

### ⚙️ Miscellaneous Tasks

- Added release workflow

## [0.1.1] - 2026-09-24

### 🚀 Features

- Added CHANGELOG.md

### 🐛 Bug Fixes

- Deleted README.pdf
- Updated cacha20 dependency

### 🚜 Refactor

- Consolidated read_u64_list/read_u8_list into a generic read_typed_list

### 📚 Documentation

- Updated README, fixed rustdoc warnings, added cargo doc job to CI workflow
- Minor changes to README
- Updated Cargo.toml
- Updated README
- Added crate-level and module-level documentation, along with missing item docs

### 🧪 Testing

- Added unit test coverage for modules filter, metadata, svs, stain, extraction
- Added unit test coverage for slide and dataset modules, fixed DatasetReport stats being tied to report generation

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
- Added a cross-platfrom release workflow
