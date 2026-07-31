//! Basic tissue filtering methods for tiles

use image::{DynamicImage, GrayImage};

// Builds a histogram from a given grayscale image
pub(crate) fn grayscale_histogram(gray_image: &GrayImage) -> [u32; 256] {
    let mut histogram = [0u32; 256];
    for pixel in gray_image.pixels() {
        // pixel.0[0] holds the intensity
        histogram[pixel.0[0] as usize] += 1;
    }
    histogram
}

/// Computes the otsu threshold for a given grayscale histogram.
///
/// The threshold is determined by maximizing the inter-variance
/// beetween background and foreground
/// This implementation is largely inspired by the C++ implementation
/// in this article: https://learnopencv.com/otsu-thresholding-with-opencv/
///
/// Operating on a histogram (rather than an image directly) allows the
/// histograms of several images/tiles to be summed before a single,
/// shared threshold is derived from them.
///
/// # Arguments
/// * `histogram` - A 256-bin grayscale intensity histogram.
///
/// # Returns
/// The threshold as an [`u8`]
pub(crate) fn otsu_threshold_from_histogram(histogram: &[u32; 256]) -> u8 {
    // total pixel count
    let total = histogram.iter().sum::<u32>() as f64;
    if total == 0.0 {
        return 0;
    };

    let intensity_weighted_sum: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, &count)| i as f64 * count as f64)
        .sum();

    let mut background_intensity = 0.0;
    let mut background_weight = 0.0;
    let mut max_variance = 0.0;
    let mut threshold = 0u8;

    for (intensity, &count) in histogram.iter().enumerate() {
        background_weight += count as f64;

        let foreground_weight = total - background_weight;

        if background_weight == 0.0 {
            continue;
        };
        if foreground_weight == 0.0 {
            break;
        };

        background_intensity += intensity as f64 * count as f64;

        let background_mean = background_intensity / background_weight;
        let foreground_mean = (intensity_weighted_sum - background_intensity) / foreground_weight;

        let inter_variance =
            background_weight * foreground_weight * (background_mean - foreground_mean).powi(2);

        if inter_variance > max_variance {
            max_variance = inter_variance;
            threshold = intensity as u8;
        }
    }

    threshold
}

/// Computes the fraction of pixels in `gray_image` at or below `threshold`.
fn tissue_fraction_below(gray_image: &GrayImage, threshold: u8) -> f32 {
    let total_pixels = gray_image.iter().len();
    if total_pixels == 0 {
        return 0.0;
    }

    // not sure if this should be < or <=
    let tissue_count = gray_image.pixels().filter(|&p| p.0[0] <= threshold).count();
    tissue_count as f32 / total_pixels as f32
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TissueMask {
    threshold: u8,
    tissue_fraction: f32,
}

impl TissueMask {
    /// Computes a tissue mask by deriving an Otsu threshold from this
    /// image's own pixels.
    ///
    /// Note that this can misclassify an isolated, near-uniform tile:
    /// Otsu always splits a histogram into two classes, even when the
    /// tile contains no real tissue/background boundary at all. Prefer
    /// [`TissueMask::compute_with_threshold`] with a threshold derived
    /// from a larger, representative region (e.g. a whole-slide overview)
    /// when classifying many tiles from the same slide.
    pub fn compute(image: &DynamicImage) -> Self {
        let gray_image: GrayImage = image.to_luma8();
        let histogram = grayscale_histogram(&gray_image);
        let threshold = otsu_threshold_from_histogram(&histogram);

        Self::compute_with_threshold(image, threshold)
    }

    /// Computes a tissue mask using an externally-supplied Otsu threshold,
    /// rather than deriving one from this image's own pixels.
    pub fn compute_with_threshold(image: &DynamicImage, threshold: u8) -> Self {
        let gray_image: GrayImage = image.to_luma8();
        let tissue_fraction = tissue_fraction_below(&gray_image, threshold);

        Self {
            threshold,
            tissue_fraction,
        }
    }

    pub fn threshold(&self) -> u8 {
        self.threshold
    }

    pub fn tissue_fraction(&self) -> f32 {
        self.tissue_fraction
    }

    // Returns whether the tissue fraction exceeds the minimal tissue fraction
    pub fn has_more_than_min_tissue(&self, min_tissue_fraction: f32) -> bool {
        self.tissue_fraction >= min_tissue_fraction
    }
}
