//! Pluggable importers: aozora, html, text, ocr.
//!
//! Phase 1 added the source resolver. Phase 2 adds an `Importer` trait
//! and the AozoraEpub3-jar-based pipeline that turns SJIS .txt into a
//! ready-to-read EPUB.

pub mod aozora;

use jp_core::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub epub_path: PathBuf,
    /// Stable identifier for the source (`aozora:<work_id>`).
    pub source_id: String,
    pub title: String,
    pub author: Option<String>,
    /// UTF-8 sidecar of the source text, when one was produced. Used by
    /// the tokenizer / vocab pipeline in later phases.
    pub raw_text_path: Option<PathBuf>,
}

/// Common interface implemented by aozora / web / OCR importers.
/// Each maps an input descriptor to an EPUB on disk.
pub trait Importer {
    type Input;

    fn import(
        &self,
        input: Self::Input,
        out_dir: &Path,
    ) -> impl std::future::Future<Output = Result<ImportResult>> + Send;
}
