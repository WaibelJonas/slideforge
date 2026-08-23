//! Writes a `manifest.csv` (slide, tile_x, tile_y) alongside a dataset
//! extraction run, using a closure as an argument for Dataset::extract

use std::env;
use std::io::Write;
use std::sync::Mutex;

use slideforge::ExtractionOptions;
use slideforge::dataset::{Dataset, DatasetOutputs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let mut args = env::args().skip(1);
    let dataset_dir = args.next().unwrap_or_else(|| "assets/dataset".to_string());
    let output_root = args
        .next()
        .unwrap_or_else(|| "assets/dataset_output".to_string());

    let dataset = Dataset::from_dir(&dataset_dir)?;
    println!("Found {} slide(s) in {dataset_dir}", dataset.len());

    std::fs::create_dir_all(&output_root)?;
    let manifest = Mutex::new(std::fs::File::create(format!(
        "{output_root}/manifest.csv"
    ))?);
    writeln!(manifest.lock().unwrap(), "slide,tile_x,tile_y")?;

    let options = ExtractionOptions::parallel()
        .with_min_tissue_fraction(0.2)
        .with_target_mpp(5.0);
    let outputs = DatasetOutputs::default(); // tiles/tfrecords off -- only the manifest is written

    dataset.extract(&options, &output_root, outputs, |slide, tile| {
        let name = slide.path().file_stem().unwrap().to_string_lossy();
        writeln!(
            manifest.lock().unwrap(),
            "{name},{},{}",
            tile.tile_x(),
            tile.tile_y()
        )?;
        Ok(())
    })?;

    println!("Manifest written to {output_root}/manifest.csv");
    Ok(())
}
