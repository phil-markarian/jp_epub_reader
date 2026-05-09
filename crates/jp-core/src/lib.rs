//! Shared types and errors used across the workspace.
//! No business logic; just data structures.

pub mod error;
pub mod settings;
pub mod source;

pub use error::{Error, Result};
pub use source::{SourceKind, SourceRef};
