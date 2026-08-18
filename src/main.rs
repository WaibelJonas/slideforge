use slideforge::ExtractionOptions;
use slideforge::dataset::{Dataset, DatasetOutputs};
use std::path::Path;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let dataset_path = Path::new("assets/dataset/");
    let output_root = Path::new("assets/dataset_output");

    let dataset = Dataset::from_dir(dataset_path)?;

    let extraction_options = ExtractionOptions::parallel()
        .with_min_tissue_fraction(0.2)
        .with_stain_normalization()
        .with_progress_bar()
        .with_target_mpp(2.5);

    let outputs = DatasetOutputs {
        tfrecords: true,
        tiles: false,
        tile_subdir: None,
        report_path: Some(output_root.join("report.pdf")),
    };

    dataset.extract(
        &extraction_options,
        &output_root,
        outputs,
        |_slide, _tile| Ok(()),
    )?;
    Ok(())
}
