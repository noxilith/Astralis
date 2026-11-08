//! In-app AI. Runs entirely on-device; nothing is sent anywhere.

pub mod assistant;
#[cfg(feature = "ml")]
pub mod bert;
pub mod brain;
pub mod embed;
#[cfg(feature = "ml")]
pub mod llm;
pub mod text;
pub mod tidy;

pub use brain::{Brain, BrainStatus, Cluster, DuplicatePair, RelatedNote, Summary, TagSuggestion};
pub use tidy::TidyResult;
