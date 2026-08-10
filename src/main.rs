use std::path::Path;

use slideforge::ExtractionOptions;
use slideforge::slide::Slide;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let path = Path::new("assets/dataset/CMU-1.svs");
    let slide = Slide::open(&path)?;

    let dir_name = path.file_stem().unwrap();
    let dir_name = format!("assets/{}", dir_name.to_str().unwrap());

    let mut extraction_options = ExtractionOptions::parallel();
    extraction_options = extraction_options
        .with_min_tissue_fraction(0.2)
        .with_stain_normalization()
        .with_progress_bar();

    let report = slide.extract_to_dir_with_report(1, &dir_name, &extraction_options)?;
    report.write_pdf(&slide, format!("{dir_name}/report.pdf"))?;
    Ok(())
}
