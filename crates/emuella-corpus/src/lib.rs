//! Catalogue validation and deterministic fixture generation for Emuella.

mod catalog;
mod fixtures;
mod model;

pub use catalog::{Catalogue, CatalogueError, CheckReport, InventoryReport, VerificationReport};
pub use fixtures::{GENERATED_CORE_ID, generate_pack};
pub use model::{PackManifest, ReviewState, SuiteManifest};
