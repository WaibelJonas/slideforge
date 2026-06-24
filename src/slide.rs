/// Representation of a single Whole Slide Image (WSI) along with its metadata.

use std::path::{PathBuf, Path};
use crate::metadata::Metadata;
use crate::error::WsiError;
use crate::backend::svs::read_metadata;

#[derive(Debug, Clone, PartialEq)]
/// Represents a single WSI, along with its metadata.
pub struct Slide {
    path: PathBuf,
    metadata: Metadata,
}

impl Slide {
    /// Opens a WSI from the specified path and returns a corresponding 
    /// [`Slide`] instance.
    /// 
    /// # Arguments
    /// * `path` - Path to the WSI file.
    /// 
    /// # Returns
    /// A [`Result`] containing a [`Slide`] instance on success, or a [`WsiError`] on failure.
    /// 
    /// # Errors
    /// Returns [`WsiError`] if the file cannot be opened or the metadata cannot be read.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WsiError> {
        let path = path.as_ref();

        let metadata =
            read_metadata(path)?;

        Ok(Self {
            path: path.to_path_buf(),
            metadata,
        })
    }

    /// Returns the path of the WSI.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Returns a reference to the metadata associated with the WSI.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the number of pyramid levels in the WSI.
    pub fn level_count(&self) -> usize {
        self.metadata.level_count()
    }


}