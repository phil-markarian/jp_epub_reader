use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AozoraStrategy {
    /// JDK21 fork (default once we bundle it).
    JarJdk21,
    /// Original hmdev/AozoraEpub3 jar.
    JarOriginal,
    /// Try jdk21 first, fall back to original on error or empty output.
    JarAuto,
    /// Native Rust converter (Phase 7 — not yet implemented).
    Native,
    /// Native first, fall back to jdk21 jar on error.
    NativeAuto,
}

impl Default for AozoraStrategy {
    fn default() -> Self {
        // Until we ship the JDK21 fork, JarAuto + JarOriginal yield the
        // same behavior because there's no jdk21 dir to try first.
        Self::JarAuto
    }
}
