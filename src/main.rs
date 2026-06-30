use std::path::Path;

use slideforge::slide::Slide;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let path = Path::new("assets/TCGA-B9EB312E82F6.svs");
    let slide = Slide::open(&path)?;

    let dir_name = path.file_stem().unwrap();
    let dir_name = format!("assets/{}", dir_name.to_str().unwrap());

    slide.extract_to_dir(0, dir_name)?;
    Ok(())
}
