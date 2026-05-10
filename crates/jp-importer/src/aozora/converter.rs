//! Strategy dispatcher: pick a jar (or native, eventually) and run it
//! with optional fallback to a sibling jar on failure / suspicious
//! output.

use crate::aozora::{
    jar::{self, JarPaths},
    java::JavaInfo,
    strategy::AozoraStrategy,
};
use jp_core::{Error, Result};
use std::path::{Path, PathBuf};

pub fn convert(
    strategy: AozoraStrategy,
    java: &JavaInfo,
    jars: &JarPaths,
    txt_path: &Path,
    out_dir: &Path,
) -> Result<PathBuf> {
    use AozoraStrategy::*;
    match strategy {
        JarJdk21 => run_jar_required(java, jars.jdk21.as_deref(), "jdk21", txt_path, out_dir),
        JarOriginal => run_jar_required(java, jars.original.as_deref(), "original", txt_path, out_dir),
        JarAuto => run_jar_auto(java, jars, txt_path, out_dir),
        Native => Err(Error::Invalid(
            "native converter not implemented (phase 7)".into(),
        )),
        NativeAuto => Err(Error::Invalid(
            "native+jar fallback not implemented (phase 7)".into(),
        )),
    }
}

fn run_jar_required(
    java: &JavaInfo,
    jar_dir: Option<&Path>,
    label: &str,
    txt_path: &Path,
    out_dir: &Path,
) -> Result<PathBuf> {
    let dir = jar_dir.ok_or_else(|| {
        Error::not_found(format!("AozoraEpub3 {label} jar not bundled"))
    })?;
    jar::run(java, dir, txt_path, out_dir)
}

fn run_jar_auto(
    java: &JavaInfo,
    jars: &JarPaths,
    txt_path: &Path,
    out_dir: &Path,
) -> Result<PathBuf> {
    // Try jdk21 → fallback to original. Skip whichever isn't bundled.
    let preferences: &[(&str, Option<&Path>)] = &[
        ("jdk21", jars.jdk21.as_deref()),
        ("original", jars.original.as_deref()),
    ];

    let mut last_err: Option<String> = None;
    let mut suspicious: Option<PathBuf> = None;

    for (label, dir) in preferences {
        let Some(dir) = dir else { continue };
        match jar::run(java, dir, txt_path, out_dir) {
            Ok(p) if jar::epub_looks_valid(&p) => {
                if let Some(s) = suspicious.take() {
                    if s != p {
                        let _ = std::fs::remove_file(&s);
                    }
                }
                return Ok(p);
            }
            Ok(p) => {
                tracing::warn!(label, ?p, "jar produced suspicious output, holding as fallback");
                suspicious = Some(p);
                last_err = Some(format!("{label}: output failed sanity check"));
            }
            Err(e) => {
                tracing::warn!(label, error=%e, "jar errored, trying next");
                last_err = Some(format!("{label}: {e}"));
            }
        }
    }

    if let Some(p) = suspicious {
        // No strategy produced a clean win; keep the most-recent
        // suspicious-but-extant EPUB rather than failing outright.
        tracing::warn!(?p, "no strategy passed sanity check; returning best-effort output");
        return Ok(p);
    }

    Err(Error::Other(format!(
        "all jars failed: {}",
        last_err.unwrap_or_else(|| "no jars bundled".into())
    )))
}
