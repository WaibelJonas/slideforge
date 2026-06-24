use std::path::Path;

use slideforge::slide::Slide;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let path = Path::new("assets/TCGA-B9EB312E82F6.svs");
    let slide = Slide::open(&path)?;
    let tile = slide.read_tile(0, 0, 1)?;
    dbg!(tile);
    Ok(())
}
