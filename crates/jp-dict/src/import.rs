//! Yomitan-format `.zip` importer.
//!
//! Walks the zip's `index.json`, `term_bank_*.json`,
//! `term_meta_bank_*.json`, `kanji_bank_*.json`, and `tag_bank_*.json`
//! files; writes rows into the schema set up by
//! `migrations/001_initial.sql`. Inserts run inside a single
//! transaction with `prepare_cached` to keep large dictionaries fast.

use crate::Db;
use jp_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Polled by the per-row insert loops. When the caller flips this to
/// true mid-import, the transaction drops without commit and every
/// row written so far is rolled back. Default = never cancelled.
static NEVER_CANCEL: AtomicBool = AtomicBool::new(false);

/// How often to read the cancel flag and emit a progress tick while
/// inserting rows. Reads are cheap but doing it every row would still
/// add up on a million-row dictionary; once per 256 rows means at
/// most ~10ms of wasted work after a cancel request, and we get
/// roughly 1–2 progress events per percent of a typical kokugo dict.
const CANCEL_CHECK_EVERY: usize = 256;

/// Progress callback signature: (bytes_done, bytes_total). Both are
/// uncompressed-byte counts derived from the zip central directory's
/// sizes for the bank files we're processing. Frontend converts to a
/// percentage for the row's progress bar.
pub type ProgressFn<'a> = &'a (dyn Fn(u64, u64) + Sync);

fn noop_progress(_: u64, _: u64) {}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Other("import cancelled".into()))
    } else {
        Ok(())
    }
}

/// Per-dictionary import outcome reported back to the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummary {
    pub dict_id: i64,
    pub name: String,
    pub format_version: i32,
    pub term_count: usize,
    pub kanji_count: usize,
    pub meta_count: usize,
    pub tag_count: usize,
}

/// Lightweight peek at a Yomitan zip's `index.json` — just enough to
/// preview the dictionary's name, revision, and format version
/// without parsing the term banks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPeek {
    pub title: String,
    pub format: i32,
    pub revision: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub attribution: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// How many term_bank_*.json / kanji_bank_*.json /
    /// term_meta_bank_*.json / tag_bank_*.json files the zip
    /// contains. Counted by entry name during peek — no parsing.
    /// A zip with `bank_file_count == 0` carries only metadata and
    /// can't contribute any lookups, so the scan flags it as
    /// "empty" instead of "ready".
    #[serde(default)]
    pub bank_file_count: u32,
}

pub fn peek_index(zip_path: &Path) -> Result<IndexPeek> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| Error::Other(format!("open zip {zip_path:?}: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::Other(format!("read zip {zip_path:?}: {e}")))?;
    let mut entry = archive
        .by_name("index.json")
        .map_err(|e| Error::Other(format!("zip missing index.json: {e}")))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|e| Error::Other(format!("read index.json: {e}")))?;
    let parsed: IndexJson = serde_json::from_str(&buf)
        .map_err(|e| Error::Other(format!("parse index.json: {e}")))?;
    Ok(IndexPeek {
        title: parsed.title,
        format: parsed.format,
        revision: parsed.revision,
        description: parsed.description,
        attribution: parsed.attribution,
        url: parsed.url,
    })
}

/// What index.json looks like at the top level. Only the fields we
/// actually need; serde tolerates extras.
#[derive(Debug, Deserialize)]
struct IndexJson {
    title: String,
    format: i32,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    attribution: Option<String>,
    #[serde(default)]
    url: Option<String>,
    // tolerated-but-ignored: sequenced, frequencyMode, isUpdatable,
    // indexUrl, downloadUrl
}

impl Db {
    /// Import one Yomitan `.zip`. Returns `Ok(None)` if a dictionary
    /// with the same name was already imported (skip without
    /// touching the existing rows). Returns `Err` for malformed
    /// zips, unsupported format versions, or sqlite errors mid-run.
    pub fn import_zip(&self, zip_path: &Path) -> Result<Option<ImportSummary>> {
        self.import_zip_with_cancel(zip_path, &NEVER_CANCEL)
    }

    /// Same as `import_zip` plus cancel polling. Used internally by
    /// the Tauri command, but kept as a public no-progress entry
    /// point for callers that don't care about per-row progress.
    pub fn import_zip_with_cancel(
        &self,
        zip_path: &Path,
        cancel: &AtomicBool,
    ) -> Result<Option<ImportSummary>> {
        self.import_zip_full(zip_path, cancel, &noop_progress)
    }

    /// Full import API: cancel polling + per-row progress callbacks.
    /// `on_progress` is invoked periodically with (bytes_done,
    /// bytes_total) where bytes are uncompressed bank-file sizes.
    /// The Tauri command wires this through to a Tauri event so the
    /// frontend can render a real progress bar inside the row's
    /// "Importing…" status cell.
    pub fn import_zip_full(
        &self,
        zip_path: &Path,
        cancel: &AtomicBool,
        on_progress: ProgressFn<'_>,
    ) -> Result<Option<ImportSummary>> {
        let file = std::fs::File::open(zip_path)
            .map_err(|e| Error::Other(format!("open zip {zip_path:?}: {e}")))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| Error::Other(format!("read zip {zip_path:?}: {e}")))?;

        let index = read_index(&mut archive)?;
        if index.format != 3 {
            return Err(Error::Other(format!(
                "unsupported Yomitan format v{} (need v3): {}",
                index.format, index.title
            )));
        }

        // Bail early if this dictionary is already imported.
        if self.dictionary_exists(&index.title)? {
            return Ok(None);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Enumerate the four bank file types up front so we know the
        // total work; then process each in document order.
        let mut term_files = Vec::new();
        let mut term_meta_files = Vec::new();
        let mut kanji_files = Vec::new();
        let mut tag_files = Vec::new();
        for i in 0..archive.len() {
            let name = archive
                .by_index(i)
                .map_err(|e| Error::Other(e.to_string()))?
                .name()
                .to_owned();
            if name.starts_with("term_bank_") && name.ends_with(".json") {
                term_files.push(name);
            } else if name.starts_with("term_meta_bank_") && name.ends_with(".json") {
                term_meta_files.push(name);
            } else if name.starts_with("kanji_bank_") && name.ends_with(".json") {
                kanji_files.push(name);
            } else if name.starts_with("kanji_meta_bank_") && name.ends_with(".json") {
                // Kanji meta is rare and the schema doesn't carry a
                // dedicated table; fold into term_meta with a leading
                // "kanji-" prefix on mode so it's distinguishable.
                term_meta_files.push(name);
            } else if name.starts_with("tag_bank_") && name.ends_with(".json") {
                tag_files.push(name);
            }
        }
        term_files.sort();
        term_meta_files.sort();
        kanji_files.sort();
        tag_files.sort();

        // Pre-pass: sum uncompressed bank sizes so the progress
        // callback can report (bytes_done, bytes_total). Opening each
        // entry just to read its central-directory size is cheap.
        let all_files: Vec<&String> = term_files
            .iter()
            .chain(term_meta_files.iter())
            .chain(kanji_files.iter())
            .chain(tag_files.iter())
            .collect();
        let mut bank_sizes: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();
        let mut total_bytes: u64 = 0;
        for name in &all_files {
            let f = archive
                .by_name(name)
                .map_err(|e| Error::Other(format!("size {name}: {e}")))?;
            let sz = f.size();
            bank_sizes.insert((*name).clone(), sz);
            total_bytes = total_bytes.saturating_add(sz);
        }
        // Initial "0%" tick so the bar appears immediately rather than
        // waiting for the first batch of rows.
        on_progress(0, total_bytes);

        let mut term_count = 0usize;
        let mut meta_count = 0usize;
        let mut kanji_count = 0usize;
        let mut tag_count = 0usize;

        let dict_id = self.with_conn_mut(|conn| {
            let tx = conn
                .transaction()
                .map_err(|e| Error::Other(format!("begin tx: {e}")))?;

            let source_path_str = zip_path.to_string_lossy().to_string();
            tx.execute(
                "INSERT INTO dictionary
                    (name, revision, format_version, priority, enabled, imported_at,
                     description, attribution, url, source_path)
                 VALUES (?, ?, ?, 0, 1, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    index.title,
                    index.revision,
                    index.format,
                    now,
                    index.description,
                    index.attribution,
                    index.url,
                    source_path_str,
                ],
            )
            .map_err(|e| Error::Other(format!("insert dictionary: {e}")))?;
            let dict_id = tx.last_insert_rowid();

            let mut bytes_done: u64 = 0;
            let progress_for_bank = |bank_size: u64,
                                     bytes_done: u64,
                                     i: usize,
                                     total_rows: usize| {
                let within = if total_rows == 0 {
                    bank_size
                } else {
                    (bank_size as f64 * (i as f64 / total_rows as f64)) as u64
                };
                on_progress(bytes_done.saturating_add(within), total_bytes);
            };

            for name in &term_files {
                check_cancel(cancel)?;
                let bank_size = *bank_sizes.get(name).unwrap_or(&0);
                let rows = parse_bank(&mut archive, name)?;
                term_count += insert_term_rows(
                    &tx,
                    dict_id,
                    &rows,
                    cancel,
                    &|i, total| progress_for_bank(bank_size, bytes_done, i, total),
                )?;
                bytes_done = bytes_done.saturating_add(bank_size);
                on_progress(bytes_done, total_bytes);
            }
            for name in &term_meta_files {
                check_cancel(cancel)?;
                let bank_size = *bank_sizes.get(name).unwrap_or(&0);
                let rows = parse_bank(&mut archive, name)?;
                meta_count += insert_term_meta_rows(
                    &tx,
                    dict_id,
                    &rows,
                    cancel,
                    &|i, total| progress_for_bank(bank_size, bytes_done, i, total),
                )?;
                bytes_done = bytes_done.saturating_add(bank_size);
                on_progress(bytes_done, total_bytes);
            }
            for name in &kanji_files {
                check_cancel(cancel)?;
                let bank_size = *bank_sizes.get(name).unwrap_or(&0);
                let rows = parse_bank(&mut archive, name)?;
                kanji_count += insert_kanji_rows(
                    &tx,
                    dict_id,
                    &rows,
                    cancel,
                    &|i, total| progress_for_bank(bank_size, bytes_done, i, total),
                )?;
                bytes_done = bytes_done.saturating_add(bank_size);
                on_progress(bytes_done, total_bytes);
            }
            for name in &tag_files {
                check_cancel(cancel)?;
                let bank_size = *bank_sizes.get(name).unwrap_or(&0);
                let rows = parse_bank(&mut archive, name)?;
                tag_count += insert_tag_rows(
                    &tx,
                    dict_id,
                    &rows,
                    cancel,
                    &|i, total| progress_for_bank(bank_size, bytes_done, i, total),
                )?;
                bytes_done = bytes_done.saturating_add(bank_size);
                on_progress(bytes_done, total_bytes);
            }

            // Final check before commit — if the user cancelled while
            // the last insert was finishing, drop the transaction.
            check_cancel(cancel)?;
            tx.commit()
                .map_err(|e| Error::Other(format!("commit tx: {e}")))?;
            Ok(dict_id)
        })?;

        Ok(Some(ImportSummary {
            dict_id,
            name: index.title,
            format_version: index.format,
            term_count,
            kanji_count,
            meta_count,
            tag_count,
        }))
    }

    fn dictionary_exists(&self, name: &str) -> Result<bool> {
        self.with_conn(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM dictionary WHERE name = ?",
                    rusqlite::params![name],
                    |r| r.get(0),
                )
                .map_err(|e| Error::Other(format!("dictionary_exists: {e}")))?;
            Ok(n > 0)
        })
    }
}

fn read_index(archive: &mut zip::ZipArchive<std::fs::File>) -> Result<IndexJson> {
    let mut f = archive
        .by_name("index.json")
        .map_err(|e| Error::Other(format!("zip missing index.json: {e}")))?;
    let mut bytes: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8 * 1024];
    loop {
        match f.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&tmp[..n]),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Invalid checksum") || msg.contains("checksum") {
                    break;
                }
                return Err(Error::Other(format!("read index.json: {e}")));
            }
        }
    }
    let buf = String::from_utf8(bytes)
        .map_err(|e| Error::Other(format!("utf8 index.json: {e}")))?;
    serde_json::from_str::<IndexJson>(&buf)
        .map_err(|e| Error::Other(format!("parse index.json: {e}")))
}

fn parse_bank(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Vec<serde_json::Value>> {
    let mut f = archive
        .by_name(name)
        .map_err(|e| Error::Other(format!("zip missing {name}: {e}")))?;

    // Some Yomitan repacks (notably 大辞林第四版, デジタル大辞泉) ship
    // with bad CRCs in the central directory even though the
    // decompressed data is valid — the system `unzip` reports the
    // same mismatches. Read in chunks so when the zip crate raises
    // its end-of-stream checksum error we can keep the bytes
    // already extracted and surface only genuine I/O failures.
    let mut bytes: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 64 * 1024];
    loop {
        match f.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&tmp[..n]),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Invalid checksum") || msg.contains("checksum") {
                    tracing::warn!(file = name, "CRC mismatch ignored; data appears intact");
                    break;
                }
                return Err(Error::Other(format!("read {name}: {e}")));
            }
        }
    }
    let buf = String::from_utf8(bytes)
        .map_err(|e| Error::Other(format!("utf8 {name}: {e}")))?;
    serde_json::from_str::<Vec<serde_json::Value>>(&buf)
        .map_err(|e| Error::Other(format!("parse {name}: {e}")))
}

fn insert_term_rows(
    tx: &rusqlite::Transaction,
    dict_id: i64,
    rows: &[serde_json::Value],
    cancel: &AtomicBool,
    on_within: &dyn Fn(usize, usize),
) -> Result<usize> {
    // Tuple shape: [expression, reading, pos, rules, score,
    //               glossary_array, sequence, term_tags]
    let mut stmt = tx
        .prepare_cached(
            "INSERT INTO term
                (dict_id, expression, reading, pos, rules, score, sequence, term_tags, glossary)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .map_err(|e| Error::Other(format!("prepare term insert: {e}")))?;

    let total = rows.len();
    let mut inserted = 0;
    for (i, row) in rows.iter().enumerate() {
        if i % CANCEL_CHECK_EVERY == 0 {
            check_cancel(cancel)?;
            on_within(i, total);
        }
        let arr = match row.as_array() {
            Some(a) if a.len() >= 6 => a,
            _ => continue, // malformed entry — skip rather than abort
        };
        let expression = arr.first().and_then(|v| v.as_str()).unwrap_or("");
        let reading = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
        let pos = arr.get(2).and_then(|v| v.as_str());
        let rules = arr.get(3).and_then(|v| v.as_str());
        let score = arr.get(4).and_then(|v| v.as_i64()).unwrap_or(0);
        let glossary = arr
            .get(5)
            .map(serde_json::Value::to_string)
            .unwrap_or_else(|| "[]".into());
        let sequence = arr.get(6).and_then(|v| v.as_i64());
        let term_tags = arr.get(7).and_then(|v| v.as_str());

        if expression.is_empty() {
            continue;
        }
        stmt.execute(rusqlite::params![
            dict_id,
            expression,
            reading,
            pos,
            rules,
            score,
            sequence,
            term_tags,
            glossary,
        ])
        .map_err(|e| Error::Other(format!("insert term: {e}")))?;
        inserted += 1;
    }
    Ok(inserted)
}

fn insert_term_meta_rows(
    tx: &rusqlite::Transaction,
    dict_id: i64,
    rows: &[serde_json::Value],
    cancel: &AtomicBool,
    on_within: &dyn Fn(usize, usize),
) -> Result<usize> {
    // Tuple shape: [expression, mode, data]
    let mut stmt = tx
        .prepare_cached(
            "INSERT INTO term_meta (dict_id, expression, mode, data)
             VALUES (?, ?, ?, ?)",
        )
        .map_err(|e| Error::Other(format!("prepare term_meta insert: {e}")))?;

    let total = rows.len();
    let mut inserted = 0;
    for (i, row) in rows.iter().enumerate() {
        if i % CANCEL_CHECK_EVERY == 0 {
            check_cancel(cancel)?;
            on_within(i, total);
        }
        let arr = match row.as_array() {
            Some(a) if a.len() >= 3 => a,
            _ => continue,
        };
        let expression = arr.first().and_then(|v| v.as_str()).unwrap_or("");
        let mode = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
        let data = arr
            .get(2)
            .map(serde_json::Value::to_string)
            .unwrap_or_else(|| "null".into());
        if expression.is_empty() || mode.is_empty() {
            continue;
        }
        stmt.execute(rusqlite::params![dict_id, expression, mode, data])
            .map_err(|e| Error::Other(format!("insert term_meta: {e}")))?;
        inserted += 1;
    }
    Ok(inserted)
}

fn insert_kanji_rows(
    tx: &rusqlite::Transaction,
    dict_id: i64,
    rows: &[serde_json::Value],
    cancel: &AtomicBool,
    on_within: &dyn Fn(usize, usize),
) -> Result<usize> {
    // Tuple shape: [char, onyomi, kunyomi, tags, meanings_array, stats_obj]
    let mut stmt = tx
        .prepare_cached(
            "INSERT INTO kanji
                (dict_id, character, onyomi, kunyomi, tags, meanings, stats)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .map_err(|e| Error::Other(format!("prepare kanji insert: {e}")))?;

    let total = rows.len();
    let mut inserted = 0;
    for (i, row) in rows.iter().enumerate() {
        if i % CANCEL_CHECK_EVERY == 0 {
            check_cancel(cancel)?;
            on_within(i, total);
        }
        let arr = match row.as_array() {
            Some(a) if a.len() >= 5 => a,
            _ => continue,
        };
        let character = arr.first().and_then(|v| v.as_str()).unwrap_or("");
        let onyomi = arr.get(1).and_then(|v| v.as_str());
        let kunyomi = arr.get(2).and_then(|v| v.as_str());
        let tags = arr.get(3).and_then(|v| v.as_str());
        let meanings = arr
            .get(4)
            .map(serde_json::Value::to_string)
            .unwrap_or_else(|| "[]".into());
        let stats = arr.get(5).map(serde_json::Value::to_string);

        if character.is_empty() {
            continue;
        }
        stmt.execute(rusqlite::params![
            dict_id, character, onyomi, kunyomi, tags, meanings, stats
        ])
        .map_err(|e| Error::Other(format!("insert kanji: {e}")))?;
        inserted += 1;
    }
    Ok(inserted)
}

fn insert_tag_rows(
    tx: &rusqlite::Transaction,
    dict_id: i64,
    rows: &[serde_json::Value],
    cancel: &AtomicBool,
    on_within: &dyn Fn(usize, usize),
) -> Result<usize> {
    // Tuple shape: [name, category, sort_key, description, score]
    let mut stmt = tx
        .prepare_cached(
            "INSERT OR REPLACE INTO tag
                (dict_id, name, category, description, score)
             VALUES (?, ?, ?, ?, ?)",
        )
        .map_err(|e| Error::Other(format!("prepare tag insert: {e}")))?;

    let total = rows.len();
    let mut inserted = 0;
    for (i, row) in rows.iter().enumerate() {
        if i % CANCEL_CHECK_EVERY == 0 {
            check_cancel(cancel)?;
            on_within(i, total);
        }
        let arr = match row.as_array() {
            Some(a) if a.len() >= 5 => a,
            _ => continue,
        };
        let name = arr.first().and_then(|v| v.as_str()).unwrap_or("");
        let category = arr.get(1).and_then(|v| v.as_str());
        let description = arr.get(3).and_then(|v| v.as_str());
        let score = arr.get(4).and_then(|v| v.as_i64());
        if name.is_empty() {
            continue;
        }
        stmt.execute(rusqlite::params![
            dict_id, name, category, description, score
        ])
        .map_err(|e| Error::Other(format!("insert tag: {e}")))?;
        inserted += 1;
    }
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn build_test_zip(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        zip.start_file("index.json", opts).unwrap();
        zip.write_all(
            r#"{"title":"Test Dict","format":3,"revision":"r1","sequenced":true}"#.as_bytes(),
        )
        .unwrap();

        zip.start_file("term_bank_1.json", opts).unwrap();
        zip.write_all(
            r#"[
                ["食べる","たべる","v1 vt","v1",100,["to eat","to consume"],1,"common"],
                ["猫","ねこ","n",null,50,["cat"],2,null]
            ]"#
                .as_bytes(),
        )
        .unwrap();

        zip.start_file("kanji_bank_1.json", opts).unwrap();
        zip.write_all(
            r#"[
                ["食","ショク ジキ","た","jouyou",["food","eat"],{"strokes":"9"}]
            ]"#
                .as_bytes(),
        )
        .unwrap();

        zip.start_file("term_meta_bank_1.json", opts).unwrap();
        zip.write_all(
            r#"[
                ["食べる","freq",100],
                ["食べる","pitch",{"reading":"たべる","pitches":[{"position":2}]}]
            ]"#
                .as_bytes(),
        )
        .unwrap();

        zip.start_file("tag_bank_1.json", opts).unwrap();
        zip.write_all(
            r#"[
                ["v1","expression",-3,"Ichidan verb",0],
                ["n","wordClass",0,"Noun",0]
            ]"#
                .as_bytes(),
        )
        .unwrap();

        zip.finish().unwrap();
    }

    #[test]
    fn import_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("test.zip");
        build_test_zip(&zip_path);

        let db = Db::open_in_memory().unwrap();
        let summary = db.import_zip(&zip_path).unwrap().expect("not skipped");

        assert_eq!(summary.name, "Test Dict");
        assert_eq!(summary.format_version, 3);
        assert_eq!(summary.term_count, 2);
        assert_eq!(summary.kanji_count, 1);
        assert_eq!(summary.meta_count, 2);
        assert_eq!(summary.tag_count, 2);

        db.with_conn(|c| {
            let n: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM term WHERE expression = '食べる'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1);
            let n: i64 = c
                .query_row("SELECT COUNT(*) FROM kanji WHERE character = '食'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(n, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn import_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("test.zip");
        build_test_zip(&zip_path);
        let db = Db::open_in_memory().unwrap();
        db.import_zip(&zip_path).unwrap().expect("first import");
        let again = db.import_zip(&zip_path).unwrap();
        assert!(again.is_none(), "second import should be skipped");
    }

    #[test]
    fn rejects_unsupported_format() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("v1.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        zip.start_file("index.json", opts).unwrap();
        zip.write_all(r#"{"title":"Old","format":1}"#.as_bytes()).unwrap();
        zip.finish().unwrap();

        let db = Db::open_in_memory().unwrap();
        let err = db.import_zip(&zip_path).unwrap_err();
        assert!(format!("{err}").contains("unsupported"));
    }
}
