//! Wrap the AozoraEpub3 jar as a subprocess.
//!
//! The jar is shipped as a directory containing `AozoraEpub3.jar`,
//! `lib/`, `template/`, and the `chuki_*.txt` config files. The jar
//! reads those config files relative to its working directory, so the
//! subprocess `cwd` is set to the jar dir.

use crate::aozora::java::JavaInfo;
use jp_core::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct JarPaths {
    pub jdk21: Option<PathBuf>,
    pub original: Option<PathBuf>,
}

impl JarPaths {
    pub fn from_resource_dir(resource_dir: &Path) -> Self {
        let jdk21 = resource_dir.join("aozoraepub3-jdk21");
        let original = resource_dir.join("aozoraepub3-original");
        Self {
            jdk21: jar_dir_if_present(jdk21),
            original: jar_dir_if_present(original),
        }
    }
}

fn jar_dir_if_present(p: PathBuf) -> Option<PathBuf> {
    if p.join("AozoraEpub3.jar").exists() {
        Some(p)
    } else {
        None
    }
}

/// Run a single jar against `txt_path`, dropping the resulting `.epub`
/// into `out_dir`. Returns the output EPUB path. Blocks the calling
/// thread — wrap callers in `tokio::task::spawn_blocking` if running
/// inside an async runtime.
pub fn run(
    java: &JavaInfo,
    jar_dir: &Path,
    txt_path: &Path,
    out_dir: &Path,
) -> Result<PathBuf> {
    std::fs::create_dir_all(out_dir)?;

    let snapshot = list_epubs(out_dir);

    let txt_abs = txt_path
        .canonicalize()
        .map_err(|e| Error::Other(format!("canonicalize txt: {e}")))?;
    let out_abs = out_dir
        .canonicalize()
        .map_err(|e| Error::Other(format!("canonicalize out_dir: {e}")))?;

    let output = Command::new(&java.binary)
        .arg("-jar")
        .arg("AozoraEpub3.jar")
        .arg("-d")
        .arg(&out_abs)
        .arg(&txt_abs)
        .current_dir(jar_dir)
        .output()
        .map_err(|e| Error::Other(format!("spawn java: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(Error::Other(format!(
            "AozoraEpub3 exited {}: stderr={stderr} stdout={stdout}",
            output.status
        )));
    }

    let after = list_epubs(out_dir);
    let new_epub = after.into_iter().find(|p| !snapshot.contains(p)).ok_or_else(|| {
        let stdout = String::from_utf8_lossy(&output.stdout);
        Error::Other(format!(
            "AozoraEpub3 produced no .epub. stdout: {}",
            stdout.trim()
        ))
    })?;

    Ok(new_epub)
}

fn list_epubs(dir: &Path) -> std::collections::BTreeSet<PathBuf> {
    let mut s = std::collections::BTreeSet::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return s;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("epub") {
            s.insert(p);
        }
    }
    s
}

/// Cheap sanity check that an EPUB file looks plausibly real.
/// Returns false on missing file, sub-4KB output, malformed zip, or
/// no XHTML payload of any meaningful size.
pub fn epub_looks_valid(epub_path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(epub_path) else {
        return false;
    };
    if bytes.len() < 4096 {
        return false;
    }
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return false;
    };
    // Different EPUB toolchains stage payload under different roots:
    // AozoraEpub3 uses OPS/, modern epubcheck output uses EPUB/, older
    // tooling uses OEBPS/, some hand-rolled tools use item/. Accept any.
    for i in 0..zip.len() {
        if let Ok(f) = zip.by_index(i) {
            let n = f.name();
            let in_payload = n.starts_with("OPS/")
                || n.starts_with("EPUB/")
                || n.starts_with("OEBPS/")
                || n.starts_with("item/");
            if in_payload && n.ends_with(".xhtml") && f.size() > 200 {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jar_dir_detection() {
        let dir = std::env::temp_dir().join(format!("jar-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let original = dir.join("aozoraepub3-original");
        std::fs::create_dir_all(&original).unwrap();
        std::fs::write(original.join("AozoraEpub3.jar"), b"placeholder").unwrap();

        let p = JarPaths::from_resource_dir(&dir);
        assert!(p.original.is_some());
        assert!(p.jdk21.is_none());
    }

    #[test]
    fn epub_looks_valid_rejects_too_small() {
        let dir = std::env::temp_dir().join(format!("epub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tiny.epub");
        std::fs::write(&p, b"x").unwrap();
        assert!(!epub_looks_valid(&p));
    }

    #[test]
    fn epub_looks_valid_rejects_missing() {
        assert!(!epub_looks_valid(std::path::Path::new("/nonexistent/file.epub")));
    }
}
