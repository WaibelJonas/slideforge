//! Modular logging/observability utilities.

pub mod extraction_observer;
pub(crate) use extraction_observer::{DualObserver, ReportCollector};
pub use extraction_observer::{ExtractionObserver, ExtractionStats, ProgressBar, SimpleLogging};
