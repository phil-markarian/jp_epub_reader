//! Seed catalog of curated metadata for well-known Yomitan
//! dictionaries. Loaded from
//! `data/known_dicts.toml` at startup; lookups normalise the dict
//! title (strip bracketed prefixes / suffixes) before matching.
//!
//! Catalog entries fill in `description`, `attribution`, and `url`
//! when those fields are missing on the imported row. They never
//! overwrite values already captured from `index.json` or set by
//! the user via the notes field.

use once_cell::sync::Lazy;
use serde::Deserialize;

const CATALOG_TOML: &str = include_str!("../data/known_dicts.toml");

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attribution: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    #[serde(default)]
    entry: Vec<CatalogEntry>,
}

static ENTRIES: Lazy<Vec<CatalogEntry>> = Lazy::new(|| {
    match toml::from_str::<CatalogFile>(CATALOG_TOML) {
        Ok(f) => f.entry,
        Err(e) => {
            tracing::warn!(error = %e, "known_dicts.toml failed to parse; catalog empty");
            Vec::new()
        }
    }
});

/// Return the catalog entry that best matches `title`, or None.
///
/// Matching strategy (cheap and predictable):
///   1. Normalise the input title (lowercase, strip bracketed
///      prefixes like `[JA-JA]` and bracketed suffixes like
///      `[2025-08-18]`, collapse whitespace).
///   2. For each entry, check whether the normalised `name` OR
///      any normalised alias is a substring of the normalised
///      title (or vice versa). First hit wins.
///
/// This is intentionally loose so repackager variants and
/// version suffixes still match the canonical entry. The trade-
/// off is occasional over-matching on very generic names —
/// catalog authors should use specific terms or aliases.
pub fn lookup(title: &str) -> Option<&'static CatalogEntry> {
    let normalised = normalise(title);
    if normalised.is_empty() {
        return None;
    }
    for entry in ENTRIES.iter() {
        let name_norm = normalise(&entry.name);
        if matches(&normalised, &name_norm) {
            return Some(entry);
        }
        for alias in &entry.aliases {
            if matches(&normalised, &normalise(alias)) {
                return Some(entry);
            }
        }
    }
    None
}

fn matches(haystack: &str, needle: &str) -> bool {
    !needle.is_empty()
        && (haystack.contains(needle) || needle.contains(haystack))
}

fn normalise(s: &str) -> String {
    // Strip the most common bracketed metadata pattern around
    // dictionary titles in the user's collection:
    //   "[JA-JA] 国語辞典オンライン" → "国語辞典オンライン"
    //   "使い方の分かる 類語例解辞典[2024-05-08]" → "使い方の分かる 類語例解辞典"
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '[' | '【' | '(' | '(' | '〔' => depth += 1,
            ']' | '】' | ')' | ')' | '〕' => {
                depth = (depth - 1).max(0);
            }
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_parses() {
        // Lazy access initialises the list — if it parses we'll
        // have non-zero entries (the file ships with many).
        assert!(!ENTRIES.is_empty(), "catalog should load entries");
    }

    #[test]
    fn matches_with_bracketed_prefix() {
        let entry = lookup("[JA-JA] 国語辞典オンライン");
        assert!(entry.is_some(), "should match despite [JA-JA] prefix");
        assert_eq!(entry.unwrap().name, "国語辞典オンライン");
    }

    #[test]
    fn matches_with_version_suffix() {
        let entry = lookup("JMnedict [2026-05-06]");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().name, "JMnedict");
    }

    #[test]
    fn matches_via_alias() {
        let entry = lookup("斎藤和英大辞典");
        assert!(entry.is_some(), "should hit via 'NEW斎藤和英大辞典' alias");
    }

    #[test]
    fn no_match_for_unknown() {
        assert!(lookup("totally made up dict name").is_none());
    }
}
