//! In-app AI. Runs entirely on-device; nothing is sent anywhere.

#[cfg(feature = "ml")]
pub mod bert;
pub mod brain;
pub mod embed;
pub mod text;
pub mod tidy;

pub use brain::{Brain, BrainStatus, Cluster, DuplicatePair, RelatedNote, Summary, TagSuggestion};
pub use tidy::TidyResult;
