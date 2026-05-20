//! Yomitan-format dictionary engine: parsing, deinflection, lookup.
//!
//! Subpiece 1 (this commit) covers the dictionary store + zip
//! importer; subpieces 2–4 (tokenizer hookup, deinflection, lookup)
//! will layer on top of the schema migrations already in place.

pub mod catalog;
pub mod db;
pub mod deinflect;
pub mod dictionary;
pub mod import;
pub mod lookup;

pub use db::Db;
pub use deinflect::{ConditionFlags, Deinflector, TransformedText};
pub use dictionary::Dictionary;
pub use import::{peek_index, ImportSummary, IndexPeek};
pub use lookup::{DictEntry, KanjiEntry, KanjiHit, LookupHit};
