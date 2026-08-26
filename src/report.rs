//! Extraction report generation.
//!
//! Collects summary data from a [`Slide::extract`](crate::slide::Slide::extract)
//! run (via [`ReportCollector`](crate::logging::ReportCollector)) and renders
//! it to a single-page PDF via [`ExtractionReport::write_pdf`].

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::DynamicImage;
use printpdf::{
    Color, FontId, Mm, Op, PaintMode, ParsedFont, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Pt, RawImage, Rect, Rgb, TextItem, XObjectId, XObjectTransform,
};

use crate::error::WsiError;
use crate::extraction::{ExtractionOptions, Parallelism};
use crate::slide::Slide;

/// Liberation Sans (SIL OFL 1.1, metric-compatible with Arial). For more information, see `assets/fonts/LICENSE`
const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/LiberationSans-Regular.ttf");

const PAGE_WIDTH_MM: f32 = 210.0;
const PAGE_HEIGHT_MM: f32 = 297.0;
const MARGIN_MM: f32 = 15.0;
const CONTENT_WIDTH_MM: f32 = PAGE_WIDTH_MM - 2.0 * MARGIN_MM;
const COLUMN_WIDTH_MM: f32 = (CONTENT_WIDTH_MM - 10.0) / 2.0;

pub(crate) const MAX_SAMPLE_TILES: usize = 8;
pub(crate) const SAMPLE_TILE_PX: u32 = 256;
pub(crate) const OVERVIEW_MAX_PX: u32 = 1000;

const HISTOGRAM_BIN_COUNT: usize = 10;

const TEXT_COLOR: Rgb = Rgb {
    r: 0.1,
    g: 0.1,
    b: 0.1,
    icc_profile: None,
};
const KEPT_COLOR: Rgb = Rgb {
    r: 0.2,
    g: 0.6,
    b: 0.2,
    icc_profile: None,
};
const BAR_COLOR: Rgb = Rgb {
    r: 0.3,
    g: 0.45,
    b: 0.75,
    icc_profile: None,
};

/// A small, `Copy` snapshot of the [`ExtractionOptions`] used for a run,
/// kept independently of the (non-`Copy`) `ExtractionOptions` itself so it
/// can be stored in [`ExtractionReport`].
#[derive(Debug, Clone, Copy)]
struct OptionsSummary {
    parallelism: Parallelism,
    min_tissue_fraction: Option<f32>,
    normalize_stain: bool,
}

impl From<&ExtractionOptions> for OptionsSummary {
    fn from(options: &ExtractionOptions) -> Self {
        Self {
            parallelism: options.parallelism,
            min_tissue_fraction: options.min_tissue_fraction,
            normalize_stain: options.normalize_stain,
        }
    }
}

/// Summary of the extraction stats for a single slide
pub(crate) struct SlideSummary {
    /// Slide name
    name: String,
    /// number of kept tiles
    kept: usize,
    /// number of dropped tiles
    dropped: usize,
    overview: DynamicImage,
    tile_positions: Vec<(u32, u32, bool)>,
    level_dimensions: (u32, u32),
    tile_size: (u32, u32),
}

impl SlideSummary {
    pub(crate) fn new(
        name: String,
        kept: usize,
        dropped: usize,
        overview: DynamicImage,
        tile_positions: Vec<(u32, u32, bool)>,
        level_dimensions: (u32, u32),
        tile_size: (u32, u32),
    ) -> Self {
        Self {
            name,
            kept,
            dropped,
            overview,
            tile_positions,
            level_dimensions,
            tile_size,
        }
    }
}

/// Summary of a completed [`Slide::extract`](crate::slide::Slide::extract)
/// run: tile counts, timing, the pipeline configuration used, and sampled
/// data (tile positions, dropped-tile tissue fractions, example
/// thumbnails), for rendering an extraction report PDF via
/// [`ExtractionReport::write_pdf`].
pub struct ExtractionReport {
    level_idx: usize,
    total: usize,
    kept: usize,
    dropped: usize,
    elapsed: Duration,
    options: OptionsSummary,
    /// (tile_x, tile_y, kept)
    tile_positions: Vec<(u32, u32, bool)>,
    dropped_tissue_fractions: Vec<f32>,
    /// (tile_x, tile_y, thumbnail)
    sample_tiles: Vec<(u32, u32, DynamicImage)>,
}

impl ExtractionReport {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        level_idx: usize,
        total: usize,
        kept: usize,
        dropped: usize,
        elapsed: Duration,
        options: &ExtractionOptions,
        tile_positions: Vec<(u32, u32, bool)>,
        dropped_tissue_fractions: Vec<f32>,
        sample_tiles: Vec<(u32, u32, DynamicImage)>,
    ) -> Self {
        Self {
            level_idx,
            total,
            kept,
            dropped,
            elapsed,
            options: OptionsSummary::from(options),
            tile_positions,
            dropped_tissue_fractions,
            sample_tiles,
        }
    }

    /// Total number of tiles considered during extraction.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Number of tiles kept (passed to the extraction callback).
    pub fn kept(&self) -> usize {
        self.kept
    }

    /// Number of tiles dropped by tissue filtering.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Renders this report as a single-page PDF at `path`.
    ///
    /// `slide` must be the same [`Slide`] the report was generated from
    /// (via [`Slide::extract`]); it's used to build the overview image and
    /// to look up level dimensions for the tile grid.
    pub fn write_pdf(&self, slide: &Slide, path: impl AsRef<Path>) -> Result<(), WsiError> {
        let mut font_warnings = Vec::new();
        let font =
            ParsedFont::from_bytes(FONT_BYTES, 0, &mut font_warnings).ok_or(WsiError::Report)?;

        let mut doc = PdfDocument::new("Tile extraction report");
        let font_id = doc.add_font(&font);

        let overview = slide.build_overview_image()?;
        let overview_raw = to_embeddable_image(&overview)?;
        let (overview_px_w, overview_px_h) =
            (overview_raw.width as u32, overview_raw.height as u32);
        let overview_image_id = doc.add_image(&overview_raw);

        let sample_images = self
            .sample_tiles
            .iter()
            .map(|(tile_x, tile_y, image)| {
                let raw = to_embeddable_image(image)?;
                let (w, h) = (raw.width as u32, raw.height as u32);
                Ok((*tile_x, *tile_y, doc.add_image(&raw), w, h))
            })
            .collect::<Result<Vec<(u32, u32, XObjectId, u32, u32)>, WsiError>>()?;

        let mut ops = Vec::new();
        let y = self.write_header(&mut ops, &font_id, slide, PAGE_HEIGHT_MM - MARGIN_MM);
        let y = self.write_tables(&mut ops, &font_id, y);
        let overview_bottom = self.write_overview(
            &mut ops,
            &font_id,
            slide,
            overview_image_id,
            overview_px_w,
            overview_px_h,
            y,
        );
        let histogram_bottom = self.write_histogram(&mut ops, &font_id, y);
        let y = overview_bottom.min(histogram_bottom);

        self.write_samples(&mut ops, &font_id, &sample_images, y);

        let page = PdfPage::new(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), ops);
        doc.with_pages(vec![page]);

        let mut save_warnings = Vec::new();
        let bytes = doc.save(&PdfSaveOptions::default(), &mut save_warnings);
        std::fs::write(path, bytes)?;

        Ok(())
    }

    fn write_header(&self, ops: &mut Vec<Op>, font: &FontId, slide: &Slide, y: f32) -> f32 {
        let slide_name = slide
            .path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());

        push_text(ops, font, 18.0, MARGIN_MM, y, "Tile extraction report");
        let y = y - 8.0;
        push_text(
            ops,
            font,
            10.0,
            MARGIN_MM,
            y,
            &format!("Slide: {slide_name}"),
        );
        let y = y - 5.0;
        push_text(
            ops,
            font,
            10.0,
            MARGIN_MM,
            y,
            &format!("Generated: {}", format_utc_now()),
        );
        let y = y - 5.0;
        push_text(
            ops,
            font,
            10.0,
            MARGIN_MM,
            y,
            &format!(
                "Level {} | Elapsed {:.1}s",
                self.level_idx,
                self.elapsed.as_secs_f32()
            ),
        );
        y - 8.0
    }

    fn write_tables(&self, ops: &mut Vec<Op>, font: &FontId, y: f32) -> f32 {
        let left_x = MARGIN_MM;
        let right_x = MARGIN_MM + COLUMN_WIDTH_MM + 10.0;

        push_text(ops, font, 12.0, left_x, y, "Pipeline");
        push_text(ops, font, 12.0, right_x, y, "Stats");
        let mut left_y = y - 6.0;
        let mut right_y = y - 6.0;

        let parallelism = match self.options.parallelism {
            Parallelism::Sequential => "sequential".to_string(),
            Parallelism::Parallel(None) => "parallel (default pool)".to_string(),
            Parallelism::Parallel(Some(n)) => format!("parallel ({n} threads)"),
        };

        let pipeline_lines = [
            format!("Level: {}", self.level_idx),
            format!("Parallelism: {parallelism}"),
            format!(
                "Min tissue fraction: {}",
                self.options
                    .min_tissue_fraction
                    .map(|f| format!("{f:.2}"))
                    .unwrap_or_else(|| "disabled".to_string())
            ),
            format!(
                "Stain normalization: {}",
                if self.options.normalize_stain {
                    "on"
                } else {
                    "off"
                }
            ),
        ];
        for line in &pipeline_lines {
            push_text(ops, font, 10.0, left_x, left_y, line);
            left_y -= 5.0;
        }

        let stats_lines = [
            format!("Total tiles considered: {}", self.total),
            format!("Kept: {}", self.kept),
            format!("Dropped: {}", self.dropped),
            format!("Elapsed: {:.1}s", self.elapsed.as_secs_f32()),
        ];
        for line in &stats_lines {
            push_text(ops, font, 10.0, right_x, right_y, line);
            right_y -= 5.0;
        }

        left_y.min(right_y) - 6.0
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_kept_tile_grid(
        ops: &mut Vec<Op>,
        tile_positions: &[(u32, u32, bool)],
        tile_w: f32,
        tile_h: f32,
        level_w: f32,
        level_h: f32,
        x0: f32,
        y0: f32,
        w_mm: f32,
        h_mm: f32,
    ) {
        ops.push(Op::SetOutlineThickness { pt: Pt(0.4) });
        ops.push(Op::SetOutlineColor {
            col: Color::Rgb(KEPT_COLOR),
        });

        for &(tile_x, tile_y, kept) in tile_positions {
            if !kept {
                continue;
            }

            let (fx0, fy0, fx1, fy1) =
                tile_fractional_bounds(tile_x, tile_y, tile_w, tile_h, level_w, level_h);

            let rect_x = x0 + fx0 * w_mm;
            let rect_w = (fx1 - fx0) * w_mm;
            let rect_y = y0 + (1.0 - fy1) * h_mm;
            let rect_h = (fy1 - fy0) * h_mm;

            ops.push(Op::DrawRectangle {
                rectangle: Rect {
                    x: Mm(rect_x).into_pt(),
                    y: Mm(rect_y).into_pt(),
                    width: Mm(rect_w).into_pt(),
                    height: Mm(rect_h).into_pt(),
                    mode: Some(PaintMode::Stroke),
                    winding_order: None,
                },
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_overview(
        &self,
        ops: &mut Vec<Op>,
        font: &FontId,
        slide: &Slide,
        image_id: XObjectId,
        px_w: u32,
        px_h: u32,
        y: f32,
    ) -> f32 {
        let panel_w = COLUMN_WIDTH_MM;
        let panel_h = 80.0;
        let x0 = MARGIN_MM;

        push_text(ops, font, 11.0, x0, y, "Overview (kept tiles outlined)");
        let image_top = y - 5.0;

        let (w_mm, h_mm) = fit_within(px_w as f32 / px_h as f32, panel_w, panel_h);
        let y0 = image_top - h_mm;

        ops.push(image_op(image_id, px_w, x0, y0, w_mm));

        if let Some(level) = slide.metadata().level(self.level_idx) {
            let level_w = level.dimensions().width as f32;
            let level_h = level.dimensions().height as f32;
            let tile_w = level.tile_size().width as f32;
            let tile_h = level.tile_size().height as f32;

            Self::draw_kept_tile_grid(
                ops,
                &self.tile_positions,
                tile_w,
                tile_h,
                level_w,
                level_h,
                x0,
                y0,
                w_mm,
                h_mm,
            );
        }

        y0 - 4.0
    }

    fn write_histogram(&self, ops: &mut Vec<Op>, font: &FontId, y: f32) -> f32 {
        let panel_w = COLUMN_WIDTH_MM;
        let panel_h = 80.0;
        let x0 = MARGIN_MM + COLUMN_WIDTH_MM + 10.0;

        push_text(ops, font, 11.0, x0, y, "Dropped-tile tissue fraction");
        let chart_top = y - 5.0;
        let chart_bottom = chart_top - panel_h;

        if self.dropped_tissue_fractions.is_empty() {
            push_text(
                ops,
                font,
                9.0,
                x0,
                chart_top - 10.0,
                "(tissue filtering disabled, no drops to chart)",
            );
            return chart_bottom - 4.0;
        }

        let bins = histogram_bins(&self.dropped_tissue_fractions, HISTOGRAM_BIN_COUNT);
        let max_count = bins.iter().copied().max().unwrap_or(1).max(1) as f32;

        let bar_gap = 1.0;
        let bar_w =
            (panel_w - bar_gap * (HISTOGRAM_BIN_COUNT as f32 - 1.0)) / HISTOGRAM_BIN_COUNT as f32;
        let baseline_y = chart_bottom + 8.0;

        ops.push(Op::SetFillColor {
            col: Color::Rgb(BAR_COLOR),
        });
        for (i, &count) in bins.iter().enumerate() {
            let bar_h = ((count as f32 / max_count) * (panel_h - 8.0)).max(0.2);
            let bar_x = x0 + i as f32 * (bar_w + bar_gap);

            ops.push(Op::DrawRectangle {
                rectangle: Rect {
                    x: Mm(bar_x).into_pt(),
                    y: Mm(baseline_y).into_pt(),
                    width: Mm(bar_w).into_pt(),
                    height: Mm(bar_h).into_pt(),
                    mode: Some(PaintMode::Fill),
                    winding_order: None,
                },
            });
        }

        push_text(ops, font, 7.0, x0, chart_bottom, "0.0");
        push_text(ops, font, 7.0, x0 + panel_w - 6.0, chart_bottom, "1.0");

        chart_bottom - 4.0
    }

    fn write_samples(
        &self,
        ops: &mut Vec<Op>,
        font: &FontId,
        samples: &[(u32, u32, XObjectId, u32, u32)],
        y: f32,
    ) {
        if samples.is_empty() {
            return;
        }

        push_text(ops, font, 11.0, MARGIN_MM, y, "Example tiles");
        let top = y - 5.0;

        let box_size = 20.0;
        let gap = if samples.len() > 1 {
            (CONTENT_WIDTH_MM - box_size * samples.len() as f32) / (samples.len() - 1) as f32
        } else {
            0.0
        };

        let mut x = MARGIN_MM;
        for (tile_x, tile_y, id, px_w, px_h) in samples {
            let (w_mm, h_mm) = fit_within(*px_w as f32 / *px_h as f32, box_size, box_size);
            let img_x = x + (box_size - w_mm) / 2.0;
            let img_y = top - box_size + (box_size - h_mm) / 2.0;

            ops.push(image_op(id.clone(), *px_w, img_x, img_y, w_mm));
            push_text(
                ops,
                font,
                6.0,
                x,
                top - box_size - 3.0,
                &format!("({tile_x}, {tile_y})"),
            );

            x += box_size + gap;
        }
    }
}

/// Normalizes `image` to 8-bit RGB before handing it to printpdf, so
/// embedding never fails due to an unexpected color format (e.g.
/// grayscale) reaching [`RawImage::from_dynamic_image`].
fn to_embeddable_image(image: &DynamicImage) -> Result<RawImage, WsiError> {
    RawImage::from_dynamic_image(DynamicImage::ImageRgb8(image.to_rgb8())).map_err(|err| {
        log::warn!("extract: failed to embed image in report: {err}");
        WsiError::Report
    })
}

fn push_text(ops: &mut Vec<Op>, font: &FontId, size_pt: f32, x_mm: f32, y_mm: f32, text: &str) {
    ops.push(Op::StartTextSection);
    ops.push(Op::SetFillColor {
        col: Color::Rgb(TEXT_COLOR),
    });
    ops.push(Op::SetFont {
        font: PdfFontHandle::External(font.clone()),
        size: Pt(size_pt),
    });
    ops.push(Op::SetTextCursor {
        pos: Point::new(Mm(x_mm), Mm(y_mm)),
    });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(text.to_string())],
    });
    ops.push(Op::EndTextSection);
}

/// Places an image so it is `w_mm` wide at `(x_mm, y_mm)` (bottom-left
/// corner), preserving its pixel aspect ratio via the dpi implied by
/// `px_w`.
fn image_op(id: XObjectId, px_w: u32, x_mm: f32, y_mm: f32, w_mm: f32) -> Op {
    let target_w_pt = Mm(w_mm).into_pt().0;
    let dpi = px_w as f32 * 72.0 / target_w_pt;

    Op::UseXobject {
        id,
        transform: XObjectTransform {
            translate_x: Some(Mm(x_mm).into_pt()),
            translate_y: Some(Mm(y_mm).into_pt()),
            rotate: None,
            scale_x: None,
            scale_y: None,
            dpi: Some(dpi),
            no_auto_scale: false,
        },
    }
}

/// Returns the `(width_mm, height_mm)` that fits an image of `aspect_ratio`
/// (width / height) within a `max_w_mm` x `max_h_mm` box, preserving it.
fn fit_within(aspect_ratio: f32, max_w_mm: f32, max_h_mm: f32) -> (f32, f32) {
    let h_at_max_w = max_w_mm / aspect_ratio;
    if h_at_max_w <= max_h_mm {
        (max_w_mm, h_at_max_w)
    } else {
        (max_h_mm * aspect_ratio, max_h_mm)
    }
}

/// Fractional bounds (`[0, 1]`, x left-to-right, y **top-to-bottom**) of an
/// extraction-level tile within its level's full image. Used to overlay a
/// tile grid onto a scaled-down rendering of the slide (e.g. the overview
/// image, built from a different, lower-resolution level) since both cover
/// the same fractional extent regardless of their actual pixel resolution.
fn tile_fractional_bounds(
    tile_x: u32,
    tile_y: u32,
    tile_w: f32,
    tile_h: f32,
    level_w: f32,
    level_h: f32,
) -> (f32, f32, f32, f32) {
    let x0 = (tile_x as f32 * tile_w) / level_w;
    let x1 = ((tile_x as f32 + 1.0) * tile_w).min(level_w) / level_w;
    let y0 = (tile_y as f32 * tile_h) / level_h;
    let y1 = ((tile_y as f32 + 1.0) * tile_h).min(level_h) / level_h;
    (x0, y0, x1, y1)
}

/// Bins `values` (expected in `[0, 1]`) into `bin_count` equal-width
/// buckets over `[0, 1]`, clamping out-of-range values into the first/last
/// bin.
fn histogram_bins(values: &[f32], bin_count: usize) -> Vec<usize> {
    let mut bins = vec![0usize; bin_count];
    for &v in values {
        let clamped = v.clamp(0.0, 1.0);
        let idx = ((clamped * bin_count as f32) as usize).min(bin_count - 1);
        bins[idx] += 1;
    }
    bins
}

/// Bins `values` into `bin_count` equal-width buckets over `[min, max]`
/// (derived from `values` itself). Returns `(bins, min, max)`. All values
/// land in the first bin when `min == max`.
fn histogram_bins_range(values: &[usize], bin_count: usize) -> (Vec<usize>, usize, usize) {
    let min = values.iter().copied().min().unwrap_or(0);
    let max = values.iter().copied().max().unwrap_or(0);
    let mut bins = vec![0usize; bin_count];

    if max == min {
        bins[0] = values.len();
        return (bins, min, max);
    }

    let range = (max - min) as f32;
    for &v in values {
        let idx = (((v - min) as f32 / range) * bin_count as f32) as usize;
        bins[idx.min(bin_count - 1)] += 1;
    }
    (bins, min, max)
}

/// Formats the current time as `YYYY-MM-DD HH:MM:SS UTC`, without pulling
/// in a date/time dependency. Uses Howard Hinnant's `civil_from_days`
/// algorithm to convert days-since-epoch into a proleptic Gregorian date.
fn format_utc_now() -> String {
    let total_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

pub struct DatasetReport {
    slides: Vec<SlideSummary>,
    failures: Vec<(PathBuf, WsiError)>,
    options: OptionsSummary,
    elapsed: Duration,
}

impl DatasetReport {
    pub(crate) fn new(
        slides: Vec<SlideSummary>,
        failures: Vec<(PathBuf, WsiError)>,
        options: &ExtractionOptions,
        elapsed: Duration,
    ) -> Self {
        Self {
            slides,
            failures,
            options: OptionsSummary::from(options),
            elapsed,
        }
    }

    pub fn total_slides(&self) -> usize {
        self.slides.len() + self.failures.len()
    }
    pub fn succeeded(&self) -> usize {
        self.slides.len()
    }
    pub fn total_kept(&self) -> usize {
        self.slides.iter().map(|s| s.kept).sum()
    }
    pub fn total_dropped(&self) -> usize {
        self.slides.iter().map(|s| s.dropped).sum()
    }

    /// Renders this report as a multi-page PDF at `path`: a summary page
    /// followed by pages of per-slide overviews (4 per page, 2x2 grid).
    pub fn write_pdf(&self, path: impl AsRef<Path>) -> Result<(), WsiError> {
        let mut font_warnings = Vec::new();
        let font =
            ParsedFont::from_bytes(FONT_BYTES, 0, &mut font_warnings).ok_or(WsiError::Report)?;

        let mut doc = PdfDocument::new("Dataset extraction report");
        let font_id = doc.add_font(&font);

        let slide_images = self
            .slides
            .iter()
            .map(|s| {
                let raw = to_embeddable_image(&s.overview)?;
                let (w, h) = (raw.width as u32, raw.height as u32);
                Ok((doc.add_image(&raw), w, h))
            })
            .collect::<Result<Vec<(XObjectId, u32, u32)>, WsiError>>()?;

        let mut pages = vec![PdfPage::new(
            Mm(PAGE_WIDTH_MM),
            Mm(PAGE_HEIGHT_MM),
            self.write_summary_page(&font_id),
        )];

        for (chunk_idx, chunk) in self.slides.chunks(4).enumerate() {
            let start = chunk_idx * 4;
            let images = &slide_images[start..start + chunk.len()];
            let ops = self.write_slide_grid_page(&font_id, chunk, images);
            pages.push(PdfPage::new(Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), ops));
        }

        doc.with_pages(pages);

        let mut save_warnings = Vec::new();
        let bytes = doc.save(&PdfSaveOptions::default(), &mut save_warnings);
        std::fs::write(path, bytes)?;

        Ok(())
    }

    fn write_summary_page(&self, font: &FontId) -> Vec<Op> {
        let mut ops = Vec::new();
        let y = PAGE_HEIGHT_MM - MARGIN_MM;

        push_text(
            &mut ops,
            font,
            18.0,
            MARGIN_MM,
            y,
            "Dataset extraction report",
        );
        let y = y - 8.0;
        push_text(
            &mut ops,
            font,
            10.0,
            MARGIN_MM,
            y,
            &format!("Generated: {}", format_utc_now()),
        );
        let y = y - 5.0;
        push_text(
            &mut ops,
            font,
            10.0,
            MARGIN_MM,
            y,
            &format!("Elapsed: {:.1}s", self.elapsed.as_secs_f32()),
        );
        let y = y - 10.0;

        let left_x = MARGIN_MM;
        let right_x = MARGIN_MM + COLUMN_WIDTH_MM + 10.0;

        push_text(&mut ops, font, 12.0, left_x, y, "Pipeline");
        push_text(&mut ops, font, 12.0, right_x, y, "Stats");
        let mut left_y = y - 6.0;
        let mut right_y = y - 6.0;

        let parallelism = match self.options.parallelism {
            Parallelism::Sequential => "sequential".to_string(),
            Parallelism::Parallel(None) => "parallel (default pool)".to_string(),
            Parallelism::Parallel(Some(n)) => format!("parallel ({n} threads)"),
        };
        let pipeline_lines = [
            format!("Parallelism: {parallelism}"),
            format!(
                "Min tissue fraction: {}",
                self.options
                    .min_tissue_fraction
                    .map(|f| format!("{f:.2}"))
                    .unwrap_or_else(|| "disabled".to_string())
            ),
            format!(
                "Stain normalization: {}",
                if self.options.normalize_stain {
                    "on"
                } else {
                    "off"
                }
            ),
        ];
        for line in &pipeline_lines {
            push_text(&mut ops, font, 10.0, left_x, left_y, line);
            left_y -= 5.0;
        }

        let stats_lines = [
            format!("Slides: {}", self.total_slides()),
            format!("Succeeded: {}", self.succeeded()),
            format!("Failed: {}", self.failures.len()),
            format!("Total kept tiles: {}", self.total_kept()),
            format!("Total dropped tiles: {}", self.total_dropped()),
        ];
        for line in &stats_lines {
            push_text(&mut ops, font, 10.0, right_x, right_y, line);
            right_y -= 5.0;
        }

        let y = left_y.min(right_y) - 6.0;
        let y = self.write_kept_histogram(&mut ops, font, y);

        if !self.failures.is_empty() {
            push_text(&mut ops, font, 11.0, MARGIN_MM, y, "Failures");
            let mut fy = y - 6.0;
            for (path, err) in &self.failures {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                push_text(
                    &mut ops,
                    font,
                    9.0,
                    MARGIN_MM,
                    fy,
                    &format!("{name}: {err}"),
                );
                fy -= 5.0;
            }
        }

        ops
    }

    fn write_kept_histogram(&self, ops: &mut Vec<Op>, font: &FontId, y: f32) -> f32 {
        let panel_w = CONTENT_WIDTH_MM;
        let panel_h = 50.0;

        push_text(ops, font, 11.0, MARGIN_MM, y, "Kept tiles per slide");
        let chart_top = y - 5.0;
        let chart_bottom = chart_top - panel_h;

        let counts: Vec<usize> = self.slides.iter().map(|s| s.kept).collect();
        if counts.is_empty() {
            push_text(
                ops,
                font,
                9.0,
                MARGIN_MM,
                chart_top - 10.0,
                "(no successful slides)",
            );
            return chart_bottom - 4.0;
        }

        let (bins, min, max) = histogram_bins_range(&counts, HISTOGRAM_BIN_COUNT);
        let max_count = bins.iter().copied().max().unwrap_or(1).max(1) as f32;

        let bar_gap = 1.0;
        let bar_w =
            (panel_w - bar_gap * (HISTOGRAM_BIN_COUNT as f32 - 1.0)) / HISTOGRAM_BIN_COUNT as f32;
        let baseline_y = chart_bottom + 8.0;

        ops.push(Op::SetFillColor {
            col: Color::Rgb(BAR_COLOR),
        });
        for (i, &count) in bins.iter().enumerate() {
            let bar_h = ((count as f32 / max_count) * (panel_h - 8.0)).max(0.2);
            let bar_x = MARGIN_MM + i as f32 * (bar_w + bar_gap);

            ops.push(Op::DrawRectangle {
                rectangle: Rect {
                    x: Mm(bar_x).into_pt(),
                    y: Mm(baseline_y).into_pt(),
                    width: Mm(bar_w).into_pt(),
                    height: Mm(bar_h).into_pt(),
                    mode: Some(PaintMode::Fill),
                    winding_order: None,
                },
            });
        }

        push_text(ops, font, 7.0, MARGIN_MM, chart_bottom, &format!("{min}"));
        push_text(
            ops,
            font,
            7.0,
            MARGIN_MM + panel_w - 6.0,
            chart_bottom,
            &format!("{max}"),
        );

        chart_bottom - 4.0
    }

    fn write_slide_grid_page(
        &self,
        font: &FontId,
        slides: &[SlideSummary],
        images: &[(XObjectId, u32, u32)],
    ) -> Vec<Op> {
        let mut ops = Vec::new();
        let cell_w = COLUMN_WIDTH_MM;
        let cell_h = 110.0;
        let gap = 10.0;

        for (i, (slide, (image_id, px_w, px_h))) in slides.iter().zip(images.iter()).enumerate() {
            let col = i % 2;
            let row = i / 2;
            let x0 = MARGIN_MM + col as f32 * (cell_w + gap);
            let top = PAGE_HEIGHT_MM - MARGIN_MM - row as f32 * (cell_h + gap);

            push_text(&mut ops, font, 11.0, x0, top, &slide.name);
            let caption_y = top - 5.0;
            push_text(
                &mut ops,
                font,
                8.0,
                x0,
                caption_y,
                &format!("Kept: {} | Dropped: {}", slide.kept, slide.dropped),
            );
            let image_top = caption_y - 5.0;

            let (w_mm, h_mm) = fit_within(*px_w as f32 / *px_h as f32, cell_w, cell_h - 10.0);
            let y0 = image_top - h_mm;

            ops.push(image_op(image_id.clone(), *px_w, x0, y0, w_mm));

            let (level_w, level_h) = (
                slide.level_dimensions.0 as f32,
                slide.level_dimensions.1 as f32,
            );
            let (tile_w, tile_h) = (slide.tile_size.0 as f32, slide.tile_size.1 as f32);

            ExtractionReport::draw_kept_tile_grid(
                &mut ops,
                &slide.tile_positions,
                tile_w,
                tile_h,
                level_w,
                level_h,
                x0,
                y0,
                w_mm,
                h_mm,
            );
        }

        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_fractional_bounds_maps_first_tile_to_top_left() {
        let (x0, y0, x1, y1) = tile_fractional_bounds(0, 0, 512.0, 512.0, 2048.0, 1024.0);
        assert_eq!((x0, y0), (0.0, 0.0));
        assert_eq!((x1, y1), (0.25, 0.5));
    }

    #[test]
    fn tile_fractional_bounds_clamps_boundary_tile() {
        // Last column tile only partially covers the level (padding beyond it).
        let (x0, _, x1, _) = tile_fractional_bounds(3, 0, 512.0, 512.0, 1800.0, 1024.0);
        assert!((x0 - (1536.0 / 1800.0)).abs() < 1e-6);
        assert_eq!(x1, 1.0);
    }

    #[test]
    fn histogram_bins_counts_and_clamps() {
        let bins = histogram_bins(&[0.0, 0.05, 0.15, 0.95, 1.0], 10);
        assert_eq!(bins, vec![2, 1, 0, 0, 0, 0, 0, 0, 0, 2]);
        assert_eq!(bins.iter().sum::<usize>(), 5);
    }

    #[test]
    fn fit_within_preserves_aspect_ratio() {
        let (w, h) = fit_within(2.0, 100.0, 40.0);
        assert_eq!((w, h), (80.0, 40.0));

        let (w, h) = fit_within(0.5, 100.0, 40.0);
        assert_eq!((w, h), (20.0, 40.0));
    }
}
