//! SVS metadata parser.
//!
//! This module contains utility functions for reading WSI
//! metadata from Aperio SVS files.
//!
//! SVS files can have multiple Image File Directories (IFDs) of various types:
//!
//! - PyramidLevel
//! - Thumbnail
//! - Label
//! - etc
//!
//! During parsing, only pyramid levels are converted into [`Level`] instances.
//!
//! Aperio-specific metadata such as objective magnification and microns per
//! pixel (MPP) is extracted from the `ImageDescription` TIFF tag when present.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use tiff::decoder::Decoder;
use tiff::tags::Tag;

use crate::tile::TileDirectory;
use crate::{Dimensions, Level, Metadata, TileSize, error::WsiError};

/// Represents the different possible types of IFDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageKind {
    PyramidLevel,
    Thumbnail,
    Label,
    Macro,
}

// Represents a parsed level from a SVS/TIFF IFD.
#[derive(Debug)]
struct ParsedIfd {
    description: String,
    tile_size: Option<TileSize>,
    kind: ImageKind,
}

pub struct ParsedSlide {
    pub metadata: Metadata,
    pub tile_directories: Vec<TileDirectory>,
}

// Contains additional metadata attributes derived from Aperio images.
#[derive(Debug, Default)]
struct AperioMetadata {
    objective_power: Option<f32>,
    microns_per_pixel: Option<f64>,
}

/// Classifies the image kind of a IFD from its description.
///
/// # Classification Rules
///
/// - Descriptions containing `"label"` are classified as [`ImageKind::Label`].
/// - Descriptions containing `"macro"` are classified as [`ImageKind::Macro`].
/// - Tiled images are assumed to be pyramid levels.
/// - Remaining images are treated as thumbnails.
///
/// # Arguments
/// * `description` - Description of the IFD as contained in the [`Tag::ImageDescription`] tag.
/// * `tile_size` - Optional tile size object.
///
/// # Returns
/// An [`ImageKind`] instance representing the IFDs type.
fn classify_ifd(description: &str, tile_size: Option<&TileSize>) -> ImageKind {
    let description = description.to_ascii_lowercase();

    if description.contains("label") {
        return ImageKind::Label;
    }

    if description.contains("macro") {
        return ImageKind::Macro;
    }

    if tile_size.is_some() {
        return ImageKind::PyramidLevel;
    }

    ImageKind::Thumbnail
}

/// Extracts metadata from the decoder's current IFD.
///
/// This function does not advance the decoder and only inspects the
/// currently selected image directory.
///
/// /// The resulting [`ParsedIfd`] can be used to determine whether the IFD
/// represents a pyramid level or an auxiliary image.
///
/// # Arguments
/// * `decoder` - The [`Decoder`] instance with the current IFD loaded.
///
/// # Returns
/// The resulting [`ParsedIfd`]
///
/// # Errors
/// Returns an [`WsiError`] if parsing the IFD fails.
fn parse_ifd(decoder: &mut Decoder<BufReader<File>>) -> Result<ParsedIfd, WsiError> {
    let description = decoder.get_tag_ascii_string(Tag::ImageDescription)?;

    let tile_size = match (
        decoder.get_tag_u32(Tag::TileWidth),
        decoder.get_tag_u32(Tag::TileLength),
    ) {
        (Ok(width), Ok(height)) => Some(TileSize::new(width, height)),
        _ => None,
    };

    let kind = classify_ifd(&description, tile_size.as_ref());

    Ok(ParsedIfd {
        description,
        tile_size,
        kind,
    })
}

/// Parses Aperio-specific metadata from an image description string.
///
/// Aperio scanners store slide metadata as key-value pairs inside the TIFF
/// `ImageDescription` tag.
///
/// Currently the following fields are extracted:
///
/// - `AppMag` (objective magnification)
/// - `MPP` (microns per pixel)
///
/// # Arguments
/// * `description` - Description of the IFD as contained in the [`Tag::ImageDescription`] tag.
///
/// # Returns
/// An [`AperioMetadata`] instance containing the extracted metadata.
fn parse_aperio_metadata(description: &str) -> AperioMetadata {
    let mut metadata = AperioMetadata::default();

    for field in description.split('|') {
        let field = field.trim();

        if let Some(value) = field.strip_prefix("AppMag = ") {
            metadata.objective_power = value.parse::<f32>().ok();
        }

        if let Some(value) = field.strip_prefix("MPP = ") {
            metadata.microns_per_pixel = value.parse::<f64>().ok();
        }
    }

    metadata
}

/// Constructs a [`Level`] from the decoder's current pyramid-level IFD.
///
/// The decoder must already be positioned at a tiled image representing
/// a valid pyramid level.
///
/// # Arguments
///
/// * `decoder` - Decoder positioned at a pyramid-level IFD.
/// * `level_idx` - Zero-based pyramid level index.
/// * `base_width` - Width of the highest-resolution level.
/// * `tile_size` - Tile dimensions associated with the current level.
///
/// # Returns
///
/// A populated [`Level`] instance.
///
/// # Errors
///
/// Returns [`WsiError`] if image dimensions cannot be read from the
/// current IFD.
fn read_level(
    decoder: &mut Decoder<BufReader<File>>,
    level_idx: u32,
    base_width: u32,
    tile_size: TileSize,
) -> Result<Level, WsiError> {
    let (width, height) = decoder.dimensions()?;

    let dimensions = Dimensions::new(width, height);

    let downsample_factor = base_width as f64 / width as f64;

    Ok(Level::new(
        level_idx,
        dimensions,
        tile_size,
        downsample_factor,
    ))
}

/// Opens an SVS file and initializes a TIFF decoder.
///
/// # Arguments
///
/// * `path` - Path to the SVS file.
///
/// # Returns
///
/// A TIFF decoder positioned at the first IFD.
///
/// # Errors
///
/// Returns [`WsiError`] if the file cannot be opened or the TIFF decoder
/// cannot be initialized.
pub fn get_decoder(path: &Path) -> Result<Decoder<BufReader<File>>, WsiError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    Ok(Decoder::new(reader)?)
}

/// Reads metadata from an Aperio SVS file.
///
/// The parser traverses all IFDs contained in the file and extracts:
///
/// - Pyramid levels
/// - Objective magnification (`AppMag`)
/// - Microns per pixel (`MPP`)
///
/// Auxiliary images such as thumbnails, labels, and macro images are
/// ignored.
///
/// # Arguments
///
/// * `path` - Path to the SVS file.
///
/// # Returns
///
/// A [`Metadata`] instance describing the slide pyramid and available
/// slide metadata.
///
/// # Errors
///
/// Returns [`WsiError`] if the file cannot be opened or required TIFF
/// metadata cannot be read.
pub fn read_metadata(path: &Path) -> Result<Metadata, WsiError> {
    let mut decoder = get_decoder(path)?;

    let base_dimensions = Dimensions::from_tuple(decoder.dimensions()?);

    let mut levels = Vec::new();

    let mut objective_power = None;
    let mut microns_per_pixel = None;

    loop {
        let parsed_ifd = parse_ifd(&mut decoder)?;

        // Parse Aperio metadata from the first useful
        // description we encounter.
        if objective_power.is_none() || microns_per_pixel.is_none() {
            let aperio = parse_aperio_metadata(&parsed_ifd.description);

            objective_power = objective_power.or(aperio.objective_power);

            microns_per_pixel = microns_per_pixel.or(aperio.microns_per_pixel);
        }

        if parsed_ifd.kind == ImageKind::PyramidLevel {
            let tile_size = parsed_ifd
                .tile_size
                .expect("Pyramid levels must have tile metadata");

            levels.push(read_level(
                &mut decoder,
                levels.len() as u32,
                base_dimensions.width,
                tile_size,
            )?);
        }

        if !decoder.more_images() {
            break;
        }

        decoder.next_image()?;
    }

    Ok(Metadata::new(levels, objective_power, microns_per_pixel))
}

pub fn parse_slide(
    path: &Path,
) -> Result<ParsedSlide, WsiError> {
    let mut decoder = get_decoder(path)?;

    let base_dimensions =
        Dimensions::from_tuple(decoder.dimensions()?);

    let mut levels = Vec::new();
    let mut tile_directories = Vec::new();

    let mut objective_power = None;
    let mut microns_per_pixel = None;

    loop {
        let parsed_ifd = parse_ifd(&mut decoder)?;

        // Parse Aperio metadata once.
        if objective_power.is_none()
            || microns_per_pixel.is_none()
        {
            let aperio =
                parse_aperio_metadata(
                    &parsed_ifd.description,
                );

            objective_power =
                objective_power.or(
                    aperio.objective_power,
                );

            microns_per_pixel =
                microns_per_pixel.or(
                    aperio.microns_per_pixel,
                );
        }

        if parsed_ifd.kind == ImageKind::PyramidLevel {
            let tile_size = parsed_ifd
                .tile_size
                .expect(
                    "Pyramid levels must have tile metadata",
                );

            levels.push(read_level(
                &mut decoder,
                levels.len() as u32,
                base_dimensions.width,
                tile_size,
            )?);

            tile_directories.push(
                TileDirectory::read_tile_directory(
                    &mut decoder,
                )?,
            );
        }

        if !decoder.more_images() {
            break;
        }

        decoder.next_image()?;
    }

    Ok(
        ParsedSlide {
            metadata: Metadata::new(
                levels,
                objective_power,
                microns_per_pixel,
            ),
            tile_directories,
        }
    )
}