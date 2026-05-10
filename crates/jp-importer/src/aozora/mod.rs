pub mod converter;
pub mod index;
pub mod jar;
pub mod java;
pub mod source;
pub mod strategy;

pub use index::{parse_index, AozoraWork};
pub use jar::JarPaths;
pub use java::JavaInfo;
pub use source::AozoraSource;
pub use strategy::AozoraStrategy;

use crate::{ImportResult, Importer};
use jp_core::{Error, Result};
use std::path::Path;

/// Aozora's webserver hangs on the default reqwest User-Agent and on
/// HTTP/2 negotiation. A browser UA + HTTP/1.1 only gets a snappy 200.
/// Apply the same defaults to aozorahack.org for consistency.
const BROWSER_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

pub struct AozoraImporter {
    pub source: AozoraSource,
    pub jars: JarPaths,
    pub java: JavaInfo,
    pub strategy: AozoraStrategy,
}

#[derive(Debug, Clone)]
pub struct AozoraInput {
    pub work: AozoraWork,
}

impl Importer for AozoraImporter {
    type Input = AozoraInput;

    async fn import(&self, input: AozoraInput, out_dir: &Path) -> Result<ImportResult> {
        let stem = input
            .work
            .stem
            .as_deref()
            .ok_or_else(|| Error::Invalid(format!(
                "work {} has no source URL — cannot resolve",
                input.work.work_id
            )))?;

        // 1. Resolve to SJIS .txt via the Phase 1 source layer.
        let sjis_path = self
            .source
            .resolve(input.work.author_id, stem)
            .await?;

        // 2. Per-work directory for this import.
        let work_dir = out_dir.join(input.work.work_id.to_string());
        std::fs::create_dir_all(&work_dir)?;

        // 3. UTF-8 sidecar for downstream tokenization (phase 4+).
        let bytes = std::fs::read(&sjis_path)?;
        let (utf8_cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(&bytes);
        if had_errors {
            tracing::warn!(work_id = input.work.work_id, "SJIS decode had errors");
        }
        let utf8_path = work_dir.join("source.utf8.txt");
        std::fs::write(&utf8_path, utf8_cow.as_bytes())?;

        // 4. Copy a SJIS source into the per-work dir; the jar reads
        //    from its own working directory and writes the EPUB next
        //    to the input.
        let sjis_dest = work_dir.join("source.txt");
        if !same_file(&sjis_path, &sjis_dest) {
            std::fs::copy(&sjis_path, &sjis_dest)?;
        }

        // 5. Run the jar(s) on a blocking thread; the EPUB lands in
        //    work_dir.
        let strategy = self.strategy;
        let jars = self.jars.clone();
        let java = self.java.clone();
        let work_dir_clone = work_dir.clone();
        let sjis_for_jar = sjis_dest.clone();
        let epub_path = tokio::task::spawn_blocking(move || {
            converter::convert(strategy, &java, &jars, &sjis_for_jar, &work_dir_clone)
        })
        .await
        .map_err(|e| Error::Other(format!("convert join: {e}")))??;

        Ok(ImportResult {
            epub_path,
            source_id: format!("aozora:{}", input.work.work_id),
            title: input.work.title.clone(),
            author: Some(input.work.author.clone()),
            raw_text_path: Some(utf8_path),
        })
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

pub(crate) fn aozora_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .http1_only()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Other(format!("build http client: {e}")))
}
