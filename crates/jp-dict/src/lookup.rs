//! Dictionary lookup pipeline.
//!
//! Given a chunk of Japanese text and an offset (typically where the
//! user clicked or hovered with the modifier key), this returns the
//! best dictionary entries for the word at that position.
//!
//! Algorithm — mirrors Yomitan's "scanner-based" approach (no
//! tokenizer):
//!
//! 1. Take the substring starting at the offset, up to `max_scan_len`
//!    characters.
//! 2. For decreasing lengths L = max, max-1, …, 1:
//!    a. Take the prefix of length L.
//!    b. Run the deinflector to enumerate candidate dictionary forms.
//!    c. Query the `term` table for each candidate's text.
//!    d. If any hits, collect them.
//! 3. Group hits by the source prefix length; return the longest
//!    successful match plus shorter alternatives (helps when the
//!    longer match is wrong).
//!
//! Output is sorted by (prefix length desc, candidate score desc,
//! dictionary priority desc) so the most likely interpretation
//! surfaces first in the popup.

use crate::deinflect::{Deinflector, TransformedText};
use crate::Db;
use jp_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One dictionary entry attached to a lookup result. Glossary stays
/// as raw JSON — the frontend parses it for rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictEntry {
    pub dict_id: i64,
    pub dict_name: String,
    pub expression: String,
    pub reading: String,
    pub pos: Option<String>,
    pub rules: Option<String>,
    pub score: i64,
    pub glossary_json: String,
}

/// All hits for one (source-substring, deinflected-candidate) pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupHit {
    /// The piece of the input text that the lookup matched against
    /// (e.g. "食べた" — the longest prefix that produced a dictionary
    /// hit).
    pub source: String,
    /// The deinflected form that actually matched a dictionary entry
    /// (e.g. "食べる").
    pub candidate: String,
    /// Chain of transformations applied to reach `candidate` from
    /// `source`, newest first. Empty for direct (dictionary-form)
    /// matches.
    pub inflection_chain: Vec<String>,
    pub entries: Vec<DictEntry>,
}

/// One kanji entry — the kanji-side analogue of `DictEntry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanjiEntry {
    pub dict_id: i64,
    pub dict_name: String,
    pub character: String,
    pub onyomi: Option<String>,
    pub kunyomi: Option<String>,
    pub meanings: Vec<String>,
    /// Free-form stats JSON (grade, strokes, frequency, …).
    /// Decoded at popup-render time since each dict ships its
    /// own keys.
    pub stats_json: Option<String>,
}

/// All entries we have for a single kanji character. The lookup
/// returns at most one of these (the character the user hovered);
/// the popup walks `entries` in dictionary-priority order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanjiHit {
    pub character: String,
    pub entries: Vec<KanjiEntry>,
}

impl Db {
    /// Lookup at the start of `text`. Caller is responsible for
    /// having sliced the input to start at the cursor position.
    /// `max_scan_len` caps how many UTF-8 characters we'll consider
    /// in the longest-first scan (Yomitan's default is 16; we use
    /// the same).
    pub fn lookup(&self, text: &str, max_scan_len: usize) -> Result<Vec<LookupHit>> {
        let deinflector = Deinflector::new();
        self.lookup_with(text, max_scan_len, &deinflector, None)
    }

    /// Restricted lookup: only consider dicts whose `kind` is in
    /// the passed slice (or NULL — pre-migration rows). Used by
    /// the reader's word + context lookup modes so kanji-only
    /// dicts don't drown text results.
    pub fn lookup_kinds(
        &self,
        text: &str,
        max_scan_len: usize,
        kinds: &[&str],
    ) -> Result<Vec<LookupHit>> {
        let deinflector = Deinflector::new();
        self.lookup_with(text, max_scan_len, &deinflector, Some(kinds))
    }

    /// Same as `lookup` but reuses an existing `Deinflector` —
    /// useful when the caller is doing many lookups in a row
    /// (e.g. batch mining) and doesn't want to rebuild the rule
    /// table per call. The rule table construction is small (~ms)
    /// but still wasted work in a tight loop.
    pub fn lookup_with(
        &self,
        text: &str,
        max_scan_len: usize,
        deinflector: &Deinflector,
        kinds: Option<&[&str]>,
    ) -> Result<Vec<LookupHit>> {
        if text.is_empty() {
            return Ok(Vec::new());
        }

        // Walk characters (not bytes) so multi-byte Japanese codepoints
        // are handled cleanly. The byte indices we collect let us
        // build prefixes via &text[..n] without panicking mid-codepoint.
        let char_byte_indices: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        let char_count = char_byte_indices.len();
        let limit = max_scan_len.min(char_count);
        if limit == 0 {
            return Ok(Vec::new());
        }

        // Collect hits in a BTreeMap keyed by (prefix_len) so we can
        // emit in longest-first order at the end. Within a prefix
        // length, we sort by candidate score / dict priority.
        let mut by_prefix: BTreeMap<usize, Vec<LookupHit>> = BTreeMap::new();

        for prefix_chars in (1..=limit).rev() {
            // Compute the byte boundary for the prefix.
            let end_byte = if prefix_chars == char_count {
                text.len()
            } else {
                char_byte_indices[prefix_chars]
            };
            let prefix = &text[..end_byte];

            let candidates = deinflector.transform(prefix);
            let mut prefix_hits = self.collect_hits_for_candidates(prefix, &candidates, kinds)?;
            if !prefix_hits.is_empty() {
                // Sort within this prefix length by entry score
                // descending so the most likely hit is first.
                prefix_hits.sort_by(|a, b| {
                    let a_score = a.entries.first().map(|e| e.score).unwrap_or(0);
                    let b_score = b.entries.first().map(|e| e.score).unwrap_or(0);
                    b_score.cmp(&a_score)
                });
                by_prefix.insert(prefix_chars, prefix_hits);
            }
        }

        // BTreeMap iterates ascending; reverse so the longest
        // matched prefix comes first in the output.
        let mut out: Vec<LookupHit> = Vec::new();
        for (_len, mut hits) in by_prefix.into_iter().rev() {
            out.append(&mut hits);
        }
        Ok(out)
    }

    fn collect_hits_for_candidates(
        &self,
        source: &str,
        candidates: &[TransformedText],
        kinds: Option<&[&str]>,
    ) -> Result<Vec<LookupHit>> {
        let mut hits: Vec<LookupHit> = Vec::new();
        // De-dup: the deinflector can produce the same candidate
        // text via different chains. We pick the shortest chain.
        let mut seen_candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for c in candidates {
            let chain: Vec<String> = c
                .trace
                .iter()
                .map(|f| f.transform_id.to_string())
                .collect();
            let existing = seen_candidates.get(&c.text);
            if existing.map(|v| v.len() <= chain.len()).unwrap_or(false) {
                continue;
            }
            seen_candidates.insert(c.text.clone(), chain);
        }

        for (candidate_text, inflection_chain) in seen_candidates {
            let entries = self.query_term_entries(&candidate_text, kinds)?;
            if entries.is_empty() {
                continue;
            }
            hits.push(LookupHit {
                source: source.to_string(),
                candidate: candidate_text,
                inflection_chain,
                entries,
            });
        }
        Ok(hits)
    }

    fn query_term_entries(
        &self,
        expression: &str,
        kinds: Option<&[&str]>,
    ) -> Result<Vec<DictEntry>> {
        // Dynamic-SQL: build a `(d.kind IN (?, ?, …) OR d.kind IS NULL)`
        // clause matching the kinds vec length so we can use simple
        // positional bindings instead of taking a dep on a JSON1
        // sqlite extension. NULL kind = pre-migration row, always
        // matches so old data still surfaces.
        let kind_clause = match kinds {
            Some(ks) if !ks.is_empty() => {
                let placeholders = std::iter::repeat("?")
                    .take(ks.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(" AND (d.kind IN ({placeholders}) OR d.kind IS NULL)")
            }
            _ => String::new(),
        };
        let sql = format!(
            "SELECT t.dict_id, d.name, t.expression, t.reading, t.pos,
                    t.rules, t.score, t.glossary
             FROM term t
             JOIN dictionary d ON d.id = t.dict_id
             WHERE t.expression = ? AND d.enabled = 1{kind_clause}
             ORDER BY d.priority DESC, t.score DESC, t.id ASC
             LIMIT 64",
        );
        self.with_conn(|c| {
            let mut stmt = c
                .prepare_cached(&sql)
                .map_err(|e| Error::Other(format!("prepare lookup: {e}")))?;
            // Bind expression first, then each kind in order.
            let mut params: Vec<&dyn rusqlite::ToSql> = vec![&expression];
            if let Some(ks) = kinds {
                for k in ks {
                    params.push(k);
                }
            }
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params), |r| {
                    Ok(DictEntry {
                        dict_id: r.get(0)?,
                        dict_name: r.get(1)?,
                        expression: r.get(2)?,
                        reading: r.get(3)?,
                        pos: r.get(4)?,
                        rules: r.get(5)?,
                        score: r.get(6)?,
                        glossary_json: r.get(7)?,
                    })
                })
                .map_err(|e| Error::Other(format!("lookup query: {e}")))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| Error::Other(e.to_string()))?);
            }
            Ok(out)
        })
    }

    /// Lookup the kanji character `ch` across all kanji-tagged
    /// (or pre-migration NULL) dictionaries. Returns None when
    /// no dict has the character — caller can show an empty
    /// kanji tab in that case.
    pub fn lookup_kanji(&self, ch: char) -> Result<Option<KanjiHit>> {
        let s = ch.to_string();
        let entries = self.with_conn(|c| {
            let mut stmt = c
                .prepare_cached(
                    "SELECT k.dict_id, d.name, k.character,
                            k.onyomi, k.kunyomi, k.meanings, k.stats
                     FROM kanji k
                     JOIN dictionary d ON d.id = k.dict_id
                     WHERE k.character = ? AND d.enabled = 1
                       AND (d.kind = 'kanji' OR d.kind IS NULL)
                     ORDER BY d.priority DESC, k.id ASC
                     LIMIT 32",
                )
                .map_err(|e| Error::Other(format!("prepare kanji lookup: {e}")))?;
            let rows = stmt
                .query_map(rusqlite::params![&s], |r| {
                    let meanings_raw: String = r.get(5)?;
                    let meanings: Vec<String> =
                        serde_json::from_str(&meanings_raw).unwrap_or_default();
                    Ok(KanjiEntry {
                        dict_id: r.get(0)?,
                        dict_name: r.get(1)?,
                        character: r.get(2)?,
                        onyomi: r.get(3)?,
                        kunyomi: r.get(4)?,
                        meanings,
                        stats_json: r.get(6)?,
                    })
                })
                .map_err(|e| Error::Other(format!("kanji lookup query: {e}")))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| Error::Other(e.to_string()))?);
            }
            Ok(out)
        })?;
        if entries.is_empty() {
            Ok(None)
        } else {
            Ok(Some(KanjiHit {
                character: s,
                entries,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Build a fixture dictionary with a couple of entries so we can
    /// exercise the lookup pipeline without a real Yomitan zip.
    fn seed(db: &Db) {
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO dictionary (name, format_version, priority, enabled, imported_at)
                 VALUES ('Fixture', 3, 0, 1, ?)",
                rusqlite::params![now()],
            ).map_err(|e| Error::Other(e.to_string()))?;
            let dict_id: i64 = c.last_insert_rowid();
            let mut insert = |expr: &str, reading: &str, rules: &str, gloss: &str| {
                c.execute(
                    "INSERT INTO term (dict_id, expression, reading, pos, rules, score, sequence, term_tags, glossary)
                     VALUES (?, ?, ?, NULL, ?, 100, 1, NULL, ?)",
                    rusqlite::params![dict_id, expr, reading, rules, gloss],
                ).unwrap();
            };
            insert("食べる", "たべる", "v1 vt", "[\"to eat\"]");
            insert("行く", "いく", "v5", "[\"to go\"]");
            insert("話す", "はなす", "v5 vt", "[\"to speak\"]");
            insert("高い", "たかい", "adj-i", "[\"high\", \"expensive\"]");
            insert("猫", "ねこ", "n", "[\"cat\"]");
            Ok(())
        }).unwrap();
    }

    #[test]
    fn lookup_dictionary_form() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let hits = db.lookup("猫が走る", 8).unwrap();
        let cat = hits.iter().find(|h| h.candidate == "猫").unwrap();
        assert_eq!(cat.entries[0].reading, "ねこ");
    }

    #[test]
    fn lookup_polite_past() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let hits = db.lookup("食べました。", 8).unwrap();
        let eat = hits.iter().find(|h| h.candidate == "食べる");
        assert!(
            eat.is_some(),
            "expected 食べました to resolve to 食べる; got: {:?}",
            hits.iter().map(|h| (&h.source, &h.candidate)).collect::<Vec<_>>()
        );
        let eat = eat.unwrap();
        assert!(
            eat.inflection_chain.iter().any(|id| id == "-ます"),
            "expected the chain to mention -ます; got {:?}",
            eat.inflection_chain
        );
    }

    #[test]
    fn lookup_picks_longest_match() {
        let db = Db::open_in_memory().unwrap();
        seed(&db);
        let hits = db.lookup("高くない", 8).unwrap();
        // The longest hit should be 高い (deinflected from 高くない).
        // 高 alone isn't in the fixture, so the only match is the
        // full 4-char prefix.
        assert_eq!(hits.first().unwrap().candidate, "高い");
    }

    #[test]
    fn lookup_empty_returns_empty() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.lookup("", 8).unwrap().is_empty());
    }

    /// Seed two dicts — one word, one kanji-only — then exercise
    /// the new kind-filtering paths.
    fn seed_with_kinds(db: &Db) {
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO dictionary (name, format_version, priority, enabled, imported_at, kind)
                 VALUES ('WordDict', 3, 10, 1, ?, 'word')",
                rusqlite::params![now()],
            ).unwrap();
            let word_id: i64 = c.last_insert_rowid();
            c.execute(
                "INSERT INTO term (dict_id, expression, reading, pos, rules, score, sequence, term_tags, glossary)
                 VALUES (?, '食', 'しょく', NULL, 'n', 50, 1, NULL, '[\"eat\"]')",
                rusqlite::params![word_id],
            ).unwrap();

            c.execute(
                "INSERT INTO dictionary (name, format_version, priority, enabled, imported_at, kind)
                 VALUES ('KanjiDict', 3, 5, 1, ?, 'kanji')",
                rusqlite::params![now()],
            ).unwrap();
            let kanji_id: i64 = c.last_insert_rowid();
            c.execute(
                "INSERT INTO kanji (dict_id, character, onyomi, kunyomi, tags, meanings, stats)
                 VALUES (?, '食', 'ショク ジキ', 'た', 'jouyou', '[\"food\",\"eat\"]', '{\"strokes\":\"9\"}')",
                rusqlite::params![kanji_id],
            ).unwrap();
            Ok(())
        }).unwrap();
    }

    #[test]
    fn lookup_kanji_returns_entries() {
        let db = Db::open_in_memory().unwrap();
        seed_with_kinds(&db);
        let hit = db.lookup_kanji('食').unwrap().expect("kanji entry");
        assert_eq!(hit.character, "食");
        assert_eq!(hit.entries.len(), 1);
        let e = &hit.entries[0];
        assert_eq!(e.dict_name, "KanjiDict");
        assert!(e.meanings.iter().any(|m| m == "eat"));
        assert_eq!(e.onyomi.as_deref(), Some("ショク ジキ"));
    }

    #[test]
    fn lookup_word_kinds_filter_skips_kanji_dicts() {
        let db = Db::open_in_memory().unwrap();
        seed_with_kinds(&db);
        // Without filter — the WordDict's "食" term entry should
        // surface even though the KanjiDict has nothing in `term`.
        let hits = db.lookup("食", 4).unwrap();
        assert!(hits.iter().any(|h| h.candidate == "食"));
        // With filter — only word-kind dicts should be queried,
        // which the WordDict satisfies. Kanji-only dicts never
        // had terms so they wouldn't have shown up anyway, but
        // the filter shouldn't drop the word entry either.
        let filtered = db.lookup_kinds("食", 4, &["word"]).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].entries[0].dict_name, "WordDict");
    }
}
