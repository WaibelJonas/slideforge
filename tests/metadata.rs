use slideforge::metadata::{Dimensions, Level, Metadata, TileSize};
#[test]
fn creates_dimensions() {
    let dimensions = Dimensions::new(80000, 60000);

    assert_eq!(dimensions.width, 80000);
    assert_eq!(dimensions.height, 60000);
}

#[test]
fn creates_tile_size() {
    let tile_size = TileSize::new(240, 240);

    assert_eq!(tile_size.width, 240);
    assert_eq!(tile_size.height, 240);
}

#[test]
fn creates_level_metadata() {
    let level = Level::new(
        0,
        Dimensions::new(80000, 60000),
        TileSize::new(240, 240),
        1.0,
    );

    assert_eq!(level.level(), 0);
    assert_eq!(level.dimensions().width, 80000);
    assert_eq!(level.dimensions().height, 60000);
    assert_eq!(level.tile_size().width, 240);
}

#[test]
fn metadata_reports_level_count() {
    let metadata = Metadata::new(
        vec![
            Level::new(
                0,
                Dimensions::new(80000, 60000),
                TileSize::new(240, 240),
                1.0,
            ),
            Level::new(
                1,
                Dimensions::new(40000, 30000),
                TileSize::new(240, 240),
                2.0,
            ),
        ],
        Some(20.0),
        Some(2.4),
    );

    assert_eq!(metadata.level_count(), 2);
}
