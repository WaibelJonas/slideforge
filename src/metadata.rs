//! Vendor-independent metadata representations for Whole Slide Images (WSIs).
//!
//! WSI typically consists of multiple resolution levels arranged in hierarchical
//! (pyramid) order. Each level represents the same image content at a
//! different resolution and often associated with a fixed tile size.

#[derive(Debug, Clone, PartialEq, Eq)]
/// Dimensions of a single image tile.
pub struct TileSize {
    /// Tile width in pixels.
    pub width: u32,

    /// Tile height in pixels.
    pub height: u32,
}

impl TileSize {
    /// Creates a new [`TileSize`] instance.
    pub fn new(width: u32, height: u32) -> TileSize {
        TileSize { width, height }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Dimensions of an image.
pub struct Dimensions {
    /// Image width in pixels.
    pub width: u32,

    /// Image height in pixels.
    pub height: u32,
}

impl Dimensions {
    /// Creates a new [`Dimensions`] instance.
    pub fn new(width: u32, height: u32) -> Dimensions {
        Dimensions { width, height }
    }

    /// Creates a [`Dimensions`] instance from a tuple of pixel sizes.
    ///
    /// # Arguments
    /// * `dimensions` - Width and height in pixels as a tuple.
    ///
    /// # Example
    /// ```
    /// use slideforge::metadata::Dimensions;
    ///
    /// let dimensions: (u32, u32) = (256, 256);
    /// let dimensions = Dimensions::from_tuple(dimensions);
    /// ```
    ///
    /// # Returns
    /// An [`Dimensions`] instance with the specified width and height.
    pub fn from_tuple(dimensions: (u32, u32)) -> Dimensions {
        Dimensions {
            width: dimensions.0,
            height: dimensions.1,
        }
    }

    /// Returns the total pixel count of an image with the specified dimensions.
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Returns the aspect ration of an image with the specified dimensions.
    pub fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

#[derive(Debug, Clone, PartialEq)]
/// A single resolution level within a WSI.
///
/// WSI pyramids consist of multiple levels, where each level represents
/// the same image content at a different resolution. Level 0 is always
/// the highest-resolution (base) level.
pub struct Level {
    /// Current level index.
    level: u32,

    /// Dimensions of the level.
    dimensions: Dimensions,

    /// Tile size in width and height.
    tile_size: TileSize,

    /// Downsampling factor relative to the base level.
    ///
    /// A value of:
    ///
    /// - `1.0` indicates the base level.
    /// - `2.0` indicates half-resolution.
    /// - `4.0` indicates quarter-resolution.
    /// - and so on.
    downsample_factor: f64,
}

impl Level {
    /// Creates a new pyramid level.
    pub fn new(
        level: u32,
        dimensions: Dimensions,
        tile_size: TileSize,
        downsample_factor: f64,
    ) -> Self {
        Self {
            level,
            dimensions,
            tile_size,
            downsample_factor,
        }
    }

    /// Returns the number of tiles along the x axis with the specified tile size.
    pub fn tiles_x(&self) -> u32 {
        self.dimensions.width.div_ceil(self.tile_size.width)
    }

    /// Returns the number of tiles along the y axis with the specified tile size.
    pub fn tiles_y(&self) -> u32 {
        self.dimensions.height.div_ceil(self.tile_size.height)
    }

    pub fn level(&self) -> u32 {
        self.level
    }

    pub fn dimensions(&self) -> &Dimensions {
        &self.dimensions
    }

    pub fn tile_size(&self) -> &TileSize {
        &self.tile_size
    }

    pub fn downsample_factor(&self) -> f64 {
        self.downsample_factor
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Metadata describing a WSI.
///
/// This structure contains pyramid-level information as well as optional
/// scanner metadata extracted from the source image.
pub struct Metadata {
    /// Pyramid levels ordered from highest to lowest resolution.
    levels: Vec<Level>,

    /// Objective magnification used during image acquisition.
    ///
    /// Common values include `20.0` and `40.0`.
    objective_power: Option<f32>,

    /// Physical pixel spacing expressed in microns per pixel (MPP).
    microns_per_pixel: Option<f64>,
}

impl Metadata {
    /// Creates a new metadata instance.
    pub fn new(
        levels: Vec<Level>,
        objective_power: Option<f32>,
        microns_per_pixel: Option<f64>,
    ) -> Self {
        Self {
            levels,
            objective_power,
            microns_per_pixel,
        }
    }

    /// Returns the number of pyramid levels.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Returns the highest-resolution level.
    pub fn base_level(&self) -> &Level {
        &self.levels[0]
    }

    /// Returns all pyramid levels ordered from highest to lowest resolution.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// Returns the objective magnification used during scanning.
    pub fn objective_power(&self) -> Option<f32> {
        self.objective_power
    }

    /// Returns the pixel spacing in MPP.
    pub fn microns_per_pixel(&self) -> Option<f64> {
        self.microns_per_pixel
    }
}
