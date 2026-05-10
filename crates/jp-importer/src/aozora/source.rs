//! Resolve a work_id to a path containing the SJIS .txt source.
//!
//! Two modes: a local clone of `aozorabunko_text` (offline-friendly) or
//! per-file fetch from `aozorahack.org` with on-disk caching.
//!
//! Files are SJIS regardless of what the HTTP Content-Type says. Callers
//! decode with `encoding_rs::SHIFT_JIS`.

use jp_core::{Error, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum AozoraSource {
    /// Local clone of the aozorabunko_text repo.
    LocalRepo(PathBuf),
    /// Fetch on demand from aozorahack.org, caching under `cache_dir`.
    Remote { cache_dir: PathBuf },
}

impl AozoraSource {
    /// Resolve a work to a path containing its SJIS-encoded .txt.
    /// Bytes are not decoded here — callers do that.
    pub async fn resolve(&self, author_id: u32, stem: &str) -> Result<PathBuf> {
        match self {
            Self::LocalRepo(root) => resolve_local(root, author_id, stem),
            Self::Remote { cache_dir } => resolve_remote(cache_dir, author_id, stem).await,
        }
    }
}

fn resolve_local(root: &Path, author_id: u32, stem: &str) -> Result<PathBuf> {
    let path = root
        .join("cards")
        .join(format!("{author_id:06}"))
        .join("files")
        .join(stem)
        .join(format!("{stem}.txt"));
    if !path.exists() {
        return Err(Error::not_found(format!("local repo missing: {path:?}")));
    }
    Ok(path)
}

async fn resolve_remote(cache_dir: &Path, author_id: u32, stem: &str) -> Result<PathBuf> {
    let cached = cache_dir
        .join(format!("{author_id:06}"))
        .join(format!("{stem}.txt"));
    if cached.exists() {
        return Ok(cached);
    }

    let url = format!(
        "https://aozorahack.org/aozorabunko_text/cards/{author_id:06}/files/{stem}/{stem}.txt"
    );
    tracing::info!(%url, "fetching aozora work file");

    let bytes = super::aozora_http_client()?
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::Other(format!("fetch {url}: {e}")))?
        .error_for_status()
        .map_err(|e| Error::Other(format!("fetch {url}: {e}")))?
        .bytes()
        .await
        .map_err(|e| Error::Other(format!("read {url}: {e}")))?;

    if let Some(parent) = cached.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = cached.with_extension("txt.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &cached)?;
    Ok(cached)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_resolves_existing_file() {
        let dir = std::env::temp_dir().join(format!("jp-src-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stem = "773_14560";
        let path = dir
            .join("cards")
            .join("000148")
            .join("files")
            .join(stem)
            .join(format!("{stem}.txt"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"placeholder").unwrap();

        let src = AozoraSource::LocalRepo(dir.clone());
        // Block on the future; resolve_local is sync inside.
        let got = futures_block(src.resolve(148, stem)).unwrap();
        assert_eq!(got, path);
    }

    #[test]
    fn local_missing_file_errors() {
        let dir = std::env::temp_dir().join(format!("jp-src-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = AozoraSource::LocalRepo(dir);
        let err = futures_block(src.resolve(148, "nope_0")).unwrap_err();
        assert!(format!("{err}").contains("local repo missing"));
    }

    /// Tiny ad-hoc executor — avoids pulling tokio into the dev-deps just
    /// for these two tests. Resolve_local is sync, so the future is ready
    /// on first poll.
    fn futures_block<F: std::future::Future>(mut f: F) -> F::Output {
        use std::pin::Pin;
        use std::task::{Context, Poll, Waker};
        let waker = Waker::noop();
        let mut cx = Context::from_waker(&waker);
        // SAFETY: we don't move `f` after pinning.
        let mut f = unsafe { Pin::new_unchecked(&mut f) };
        loop {
            if let Poll::Ready(out) = f.as_mut().poll(&mut cx) {
                return out;
            }
            std::thread::yield_now();
        }
    }
}
