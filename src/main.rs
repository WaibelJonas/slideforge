use slideforge::backend::svs;
use std::error::Error;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let path = Path::new("assets/TCGA-B9EB312E82F6.svs");
    let metadata = svs::read_metadata(&path)?;
    dbg!(metadata);
    Ok(())
}
