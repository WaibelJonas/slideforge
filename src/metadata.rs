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

    /// Returns the valid (non-padding) width and height of the tile at
    /// (`tile_x`, `tile_y`) in this level.
    ///
    /// Tiled image formats store tiles at a fixed size, so the last row
    /// and column of tiles in a level commonly extend past the level's
    /// real dimensions; the excess is stored as padding. This returns the
    /// portion of the tile's stored image data that corresponds to actual
    /// image content, which is always `<= tile_size` and smaller than
    /// `tile_size` only for boundary tiles.
    pub fn valid_tile_dimensions(&self, tile_x: u32, tile_y: u32) -> (u32, u32) {
        let valid_width = self
            .dimensions
            .width
            .saturating_sub(tile_x * self.tile_size.width)
            .min(self.tile_size.width);
        let valid_height = self
            .dimensions
            .height
            .saturating_sub(tile_y * self.tile_size.height)
            .min(self.tile_size.height);

        (valid_width, valid_height)
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

    /// Returns the level at the specified index.
    pub fn level(&self, index: usize) -> Option<&Level> {
        self.levels.get(index)
    }

    /// Returns the index of the level which resolution
    /// comes closest to a given target MPP value-
    ///
    /// The approximate MPP at a level is computed by
    /// multiplying the base microns_per_pixel of the WSI by the levels
    /// `downsampling_factor`
    ///
    /// # Arguments
    /// * `target_mpp` - The target MPP as an indicator of resolution
    ///
    /// # Returns
    /// An [`Option`] containing the index of an appropriate level, or [`None`]
    /// if no base MPP is known for the WSI.
    pub fn best_level_for_target_mpp(&self, target_mpp: f64) -> Option<usize> {
        let base_mpp = self.microns_per_pixel()?;
        // Approximate mpp at a given level
        let mpp_at = |level: &Level| base_mpp * level.downsample_factor();

        // Default: Return the coarsest level that qualifies
        let best_level = self
            .levels()
            .iter()
            .enumerate()
            .filter(|(_, level)| mpp_at(level) <= target_mpp)
            .max_by(|(_, a), (_, b)| mpp_at(a).total_cmp(&mpp_at(b)))
            .map(|(index, _)| index);

        // Fallback: Return the finest level
        if best_level.is_some() {
            best_level
        } else {
            self.levels()
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| mpp_at(a).total_cmp(&mpp_at(b)))
                .map(|(index, _)| index)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(index: u32, downsample_factor: f64) -> Level {
        Level::new(
            index,
            Dimensions::new(100, 100),
            TileSize::new(30, 30),
            downsample_factor,
        )
    }

    #[test]
    fn tiles_x_and_y_round_up_to_cover_the_level() {
        let exact = Level::new(0, Dimensions::new(90, 60), TileSize::new(30, 30), 1.0);
        assert_eq!(exact.tiles_x(), 3);
        assert_eq!(exact.tiles_y(), 2);

        let remainder = Level::new(0, Dimensions::new(100, 100), TileSize::new(30, 30), 1.0);
        assert_eq!(remainder.tiles_x(), 4);
        assert_eq!(remainder.tiles_y(), 4);
    }

    #[test]
    fn valid_tile_dimensions_are_full_size_in_the_interior() {
        let level = Level::new(0, Dimensions::new(100, 100), TileSize::new(30, 30), 1.0);
        assert_eq!(level.valid_tile_dimensions(0, 0), (30, 30));
    }

    #[test]
    fn valid_tile_dimensions_are_clipped_at_the_boundary() {
        let level = Level::new(0, Dimensions::new(100, 100), TileSize::new(30, 30), 1.0);
        // Last tile column/row starts at 90, but the level is only 100 wide/tall.
        assert_eq!(level.valid_tile_dimensions(3, 3), (10, 10));
    }

    #[test]
    fn best_level_for_target_mpp_is_none_without_a_base_mpp() {
        let metadata = Metadata::new(vec![level(0, 1.0)], None, None);
        assert_eq!(metadata.best_level_for_target_mpp(0.5), None);
    }

    #[test]
    fn best_level_for_target_mpp_picks_the_coarsest_qualifying_level() {
        // base_mpp = 0.25 => level mpp is 0.25, 0.5, 1.0
        let metadata = Metadata::new(
            vec![level(0, 1.0), level(1, 2.0), level(2, 4.0)],
            Some(20.0),
            Some(0.25),
        );

        assert_eq!(metadata.best_level_for_target_mpp(0.6), Some(1));
    }

    #[test]
    fn best_level_for_target_mpp_falls_back_to_the_finest_level() {
        let metadata = Metadata::new(
            vec![level(0, 1.0), level(1, 2.0), level(2, 4.0)],
            Some(20.0),
            Some(0.25),
        );

        // No level is fine enough for this target, so the finest level wins.
        assert_eq!(metadata.best_level_for_target_mpp(0.1), Some(0));
    }
}
