//! Astralis core: UI-agnostic logic shared by the desktop app and future sync/CRDT layers.

pub mod ai;
pub mod attachments;
pub mod calendar;
pub mod cloud;
pub mod error;
pub mod folders;
pub mod grammar;
pub mod history;
pub mod links;
pub mod memory;
pub mod merge;
pub mod notes;
pub mod planners;
pub mod search;
pub mod store;
pub mod sync;
pub mod tasks;
pub mod templates;
mod util;
pub mod vault;
pub mod whiteboards;

pub use error::{Error, Result};
pub use store::Store;
pub use util::{Timestamp, now_ms};

#[cfg(test)]
mod tests;
