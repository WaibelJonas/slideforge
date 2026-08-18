use slideforge::ExtractionOptions;
use slideforge::slide::Slide;
use std::path::Path;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let path = Path::new("assets/dataset/CMU-1.svs");
    let slide = Slide::open(&path)?;

    let dir_name = path.file_stem().unwrap();
    let dir_name = format!("assets/{}", dir_name.to_str().unwrap());

    let extraction_options = ExtractionOptions::parallel()
        .with_min_tissue_fraction(0.2)
        .with_stain_normalization()
        .with_progress_bar()
        .with_target_mpp(2.5)
        .with_slide_report(format!("{dir_name}/report.pdf"))
        .with_output_dir(&dir_name)
        .with_tfrecord_file(format!("{}.tfrecords", &dir_name));

    slide.extract(&extraction_options, |_tile| Ok(()))?;
    Ok(())
}
