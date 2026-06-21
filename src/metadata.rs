#[derive(Debug, Clone, PartialEq, Eq)]
/// Size of a single tile.
pub struct TileSize {
    pub width: u32,
    pub height: u32,
}

impl TileSize {
    pub fn new(width: u32, height: u32) -> TileSize {
        TileSize { width, height }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Width and height of an image.
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl Dimensions {
    pub fn new(width: u32, height: u32) -> Dimensions {
        Dimensions { width, height }
    }

    /// Returns the total pixel count of an image with this dimension.
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Returns the aspect ration of an image with this dimension.
    pub fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Single WSI level.
pub struct Level {
    level: u32,
    dimensions: Dimensions,
    tile_size: TileSize,
    downsample: f32,
}

impl Level {
    pub fn new(level: u32, dimensions: Dimensions, tile_size: TileSize, downsample: f32) -> Self {
        Self {
            level,
            dimensions,
            tile_size,
            downsample,
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

    pub fn downsample(&self) -> f32 {
        self.downsample
    }
}

#[derive(Debug, Clone, PartialEq)]
/// General information about a Whole Slide Image (WSI).
pub struct Metadata {
    pub dimensions: Dimensions,
    pub levels: Vec<Level>,
}

impl Metadata {
    pub fn new(dimensions: Dimensions, levels: Vec<Level>) -> Self {
        Self { dimensions, levels }
    }

    /// Returns the number of levels of the WSI.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Returns the base level of the WSI.
    pub fn base_level(&self) -> &Level {
        &self.levels[0]
    }

    pub fn dimensions(&self) -> &Dimensions {
        &self.dimensions
    }

    pub fn levels(&self) -> &Vec<Level> {
        &self.levels
    }
}
