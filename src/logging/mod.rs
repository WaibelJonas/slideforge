//! Modular logging/observability utilities.

pub mod extraction_observer;
pub(crate) use extraction_observer::DualObserver;
pub use extraction_observer::{
    ExtractionObserver, ExtractionStats, ProgressBar, ReportCollector, SimpleLogging,
};
