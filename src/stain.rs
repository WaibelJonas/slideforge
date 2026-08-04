//! Reinhard color/stain normalization for RGB tile images.
//!
//! Normalizes a tile's color statistics in CIE L*a*b* space (mean and
//! standard deviation per channel) to match a fixed target, so tiles
//! extracted from different slides/scanners/staining batches look more
//! consistent.
//!
//! Source Reinhard et al., "Color Transfer between Images" (2001).

use std::sync::OnceLock;

use image::{DynamicImage, Rgb, RgbImage};

/// Reference image/tissue tile that serves as the color-normalization target.
/// TODO! | Should probably be replaced by a more applicable/general purpose reference
static REFERENCE_TILE_BYTES: &[u8] = include_bytes!("../assets/reference_tile.jpg");

/// Below this standard deviation, a tile's Lab channel is treated as
/// constant to avoid dividing by zero.
const STD_EPSILON: f64 = 1e-6;

// D65 reference white.
const XN: f64 = 0.95047;
const YN: f64 = 1.0;
const ZN: f64 = 1.08883;

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn xyz_forward(t: f64) -> f64 {
    const DELTA: f64 = 6.0 / 29.0;
    if t > DELTA * DELTA * DELTA {
        t.cbrt()
    } else {
        t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
    }
}

fn xyz_inverse(t: f64) -> f64 {
    const DELTA: f64 = 6.0 / 29.0;
    if t > DELTA {
        t * t * t
    } else {
        3.0 * DELTA * DELTA * (t - 4.0 / 29.0)
    }
}

/// Converts an sRGB pixel to CIE L*a*b* (D65 white point).
fn rgb_to_lab(pixel: Rgb<u8>) -> [f64; 3] {
    let [r, g, b] = pixel.0;
    let r = srgb_to_linear(r as f64 / 255.0);
    let g = srgb_to_linear(g as f64 / 255.0);
    let b = srgb_to_linear(b as f64 / 255.0);

    let x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b;

    let fx = xyz_forward(x / XN);
    let fy = xyz_forward(y / YN);
    let fz = xyz_forward(z / ZN);

    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let b = 200.0 * (fy - fz);

    [l, a, b]
}

/// Converts a CIE L*a*b* value back to an sRGB pixel, clamping to `[0, 255]`.
fn lab_to_rgb(lab: [f64; 3]) -> Rgb<u8> {
    let [l, a, b] = lab;

    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;

    let x = XN * xyz_inverse(fx);
    let y = YN * xyz_inverse(fy);
    let z = ZN * xyz_inverse(fz);

    let r = 3.2404542 * x - 1.5371385 * y - 0.4985314 * z;
    let g = -0.9692660 * x + 1.8760108 * y + 0.0415560 * z;
    let b = 0.0556434 * x - 0.2040259 * y + 1.0572252 * z;

    let to_u8 = |c: f64| (linear_to_srgb(c).clamp(0.0, 1.0) * 255.0).round() as u8;

    Rgb([to_u8(r), to_u8(g), to_u8(b)])
}

/// Computes the per-channel mean and standard deviation of a set of Lab
/// pixels.
fn lab_mean_std(lab_pixels: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    let count = lab_pixels.len() as f64;

    let mut mean = [0.0; 3];
    for pixel in lab_pixels {
        for c in 0..3 {
            mean[c] += pixel[c];
        }
    }
    for m in &mut mean {
        *m /= count;
    }

    let mut variance = [0.0; 3];
    for pixel in lab_pixels {
        for c in 0..3 {
            let diff = pixel[c] - mean[c];
            variance[c] += diff * diff;
        }
    }

    let mut std = [0.0; 3];
    for c in 0..3 {
        std[c] = (variance[c] / count).sqrt();
    }

    (mean, std)
}

/// Returns the target CIE L*a*b* (mean, standard deviation), computed from
/// [`REFERENCE_TILE_BYTES`] on first use and cached for the process
/// lifetime.
fn target_stats() -> &'static ([f64; 3], [f64; 3]) {
    static TARGET_STATS: OnceLock<([f64; 3], [f64; 3])> = OnceLock::new();

    TARGET_STATS.get_or_init(|| {
        let reference = image::load_from_memory(REFERENCE_TILE_BYTES)
            .expect("bundled reference tile is a valid image");
        let lab_pixels: Vec<[f64; 3]> = reference
            .to_rgb8()
            .pixels()
            .map(|&p| rgb_to_lab(p))
            .collect();
        lab_mean_std(&lab_pixels)
    })
}

/// Applies Reinhard stain normalization on a given image
///
/// # Arguments
/// * `image` - Reference to a [`DynamicImage`] to normalize
///
/// # Returns
/// A stain normalized version of the original image as a [`DynamicImage`]
pub fn normalize_reinhard(image: &DynamicImage) -> DynamicImage {
    let (target_mean, target_std) = target_stats();

    let rgb_image = image.to_rgb8();
    let lab_pixels: Vec<[f64; 3]> = rgb_image.pixels().map(|&p| rgb_to_lab(p)).collect();
    let (mean, std) = lab_mean_std(&lab_pixels);

    let (width, height) = rgb_image.dimensions();
    let mut output = RgbImage::new(width, height);

    for (pixel, lab_pixel) in output.pixels_mut().zip(lab_pixels.iter()) {
        let mut normalized = [0.0; 3];
        for c in 0..3 {
            let scale = if std[c] > STD_EPSILON {
                target_std[c] / std[c]
            } else {
                1.0
            };
            normalized[c] = (lab_pixel[c] - mean[c]) * scale + target_mean[c];
        }
        *pixel = lab_to_rgb(normalized);
    }

    DynamicImage::ImageRgb8(output)
}
