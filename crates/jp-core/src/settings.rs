use crate::{Error, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// JSON-backed key/value store. Keys are dotted strings ("aozora.repo_path").
/// Values are `serde_json::Value` so callers can store strings, numbers,
/// or structured config without a per-key schema.
///
/// All writes flush to disk immediately. Cheap; settings change rarely.
#[derive(Debug, Clone)]
pub struct Settings {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    map: BTreeMap<String, Value>,
}

impl Settings {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let map = if path.exists() {
            let bytes = std::fs::read(&path)?;
            if bytes.is_empty() {
                BTreeMap::new()
            } else {
                serde_json::from_slice(&bytes)?
            }
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner { path, map })),
        })
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .map
            .get(key)
            .and_then(|v| v.as_str().map(str::to_owned))
    }

    pub fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.inner
            .lock()
            .unwrap()
            .map
            .get(key)
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn set<T: serde::Serialize>(&self, key: &str, value: T) -> Result<()> {
        let v = serde_json::to_value(value)?;
        let mut inner = self.inner.lock().unwrap();
        inner.map.insert(key.to_owned(), v);
        write_atomic(&inner.path, &inner.map)
    }

    pub fn unset(&self, key: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.map.remove(key);
        write_atomic(&inner.path, &inner.map)
    }
}

fn write_atomic(path: &Path, map: &BTreeMap<String, Value>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(map)?;
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, path).map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let dir = tempdir();
        let path = dir.join("settings.json");
        let s = Settings::load(&path).unwrap();
        s.set("aozora.repo_path", "/some/path").unwrap();
        s.set("aozora.public_only", true).unwrap();

        let s2 = Settings::load(&path).unwrap();
        assert_eq!(s2.get_string("aozora.repo_path").as_deref(), Some("/some/path"));
        assert_eq!(s2.get::<bool>("aozora.public_only"), Some(true));
    }

    #[test]
    fn missing_file_returns_empty() {
        let dir = tempdir();
        let s = Settings::load(dir.join("nope.json")).unwrap();
        assert!(s.get_string("anything").is_none());
    }

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("jp-core-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
