//! Loader for Aozora's master CSV index of works.
//!
//! Source URL is a zipped, UTF-8 CSV. We download once, cache for
//! `FRESHNESS_DAYS`, parse on demand.

use jp_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const INDEX_URL: &str =
    "https://www.aozora.gr.jp/index_pages/list_person_all_extended_utf8.zip";
const FRESHNESS_DAYS: u64 = 7;
const CACHE_FILE: &str = "aozora-index.csv";


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AozoraWork {
    pub work_id: u32,
    pub author_id: u32,
    pub title: String,
    pub title_yomi: String,
    pub author: String,
    pub author_yomi: String,
    pub copyright_active: bool,
    /// Zip filename without extension, extracted from CSV column 45.
    /// Used to resolve into the aozorabunko_text repo path.
    /// `None` when the CSV row has no text URL or an unparseable one.
    pub stem: Option<String>,
}

/// Returns the path to a cached, up-to-date CSV. Downloads if missing or stale.
pub async fn ensure_index(cache_dir: &Path) -> Result<PathBuf> {
    let csv_path = cache_dir.join(CACHE_FILE);
    if is_fresh(&csv_path)? {
        return Ok(csv_path);
    }
    download_index(cache_dir).await?;
    Ok(csv_path)
}

fn is_fresh(csv_path: &Path) -> Result<bool> {
    let Ok(meta) = std::fs::metadata(csv_path) else {
        return Ok(false);
    };
    let modified = meta.modified()?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    Ok(age < Duration::from_secs(FRESHNESS_DAYS * 24 * 60 * 60))
}

async fn download_index(cache_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(cache_dir)?;

    tracing::info!(url = INDEX_URL, "fetching aozora index");
    let client = super::aozora_http_client()?;
    let bytes = client
        .get(INDEX_URL)
        .send()
        .await
        .map_err(|e| Error::Other(format!("download index send: {}", source_chain(&e))))?
        .error_for_status()
        .map_err(|e| Error::Other(format!("download index status: {}", source_chain(&e))))?
        .bytes()
        .await
        .map_err(|e| Error::Other(format!("download index body: {}", source_chain(&e))))?;

    extract_first_csv(&bytes, &cache_dir.join(CACHE_FILE))?;
    Ok(())
}

fn source_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut s = err.to_string();
    let mut src = err.source();
    while let Some(cause) = src {
        s.push_str(" -> ");
        s.push_str(&cause.to_string());
        src = cause.source();
    }
    s
}

fn extract_first_csv(zip_bytes: &[u8], out_path: &Path) -> Result<()> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| Error::Invalid(format!("aozora zip: {e}")))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::Invalid(format!("zip entry {i}: {e}")))?;
        if entry.name().to_ascii_lowercase().ends_with(".csv") {
            let tmp = out_path.with_extension("csv.tmp");
            let mut out = std::fs::File::create(&tmp)?;
            std::io::copy(&mut entry, &mut out)?;
            std::fs::rename(&tmp, out_path)?;
            return Ok(());
        }
    }
    Err(Error::Invalid("no .csv inside aozora index zip".into()))
}

/// Parse the cached CSV into a vec. Asserts column layout via the kokoro
/// (work_id 773) sanity check — Aozora has reshuffled columns historically.
pub fn parse_index(csv_path: &Path) -> Result<Vec<AozoraWork>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(csv_path)
        .map_err(|e| Error::Other(format!("open csv {csv_path:?}: {e}")))?;

    let mut works = Vec::with_capacity(20_000);
    let mut saw_kokoro = false;

    // Real CSV layout (verified against the live Aozora extended CSV;
    // the v1 docs had several columns off by one or two):
    //
    //   0   作品ID                  work_id
    //   1   作品名                  title
    //   2   作品名読み              title_yomi
    //   10  作品著作権フラグ          copyright flag ("あり" = active)
    //   14  人物ID                  author_id
    //   15  姓                      surname
    //   16  名                      given
    //   17  姓読み                  surname_yomi
    //   18  名読み                  given_yomi
    //   45  テキストファイルURL      text URL (zip)
    for record in rdr.records() {
        let r = record.map_err(|e| Error::Other(format!("csv row: {e}")))?;

        let work_id: u32 = field(&r, 0).parse().unwrap_or(0);
        let title = field(&r, 1).to_string();
        let title_yomi = field(&r, 2).to_string();
        let copyright_active = field(&r, 10) == "あり";
        let author_id: u32 = field(&r, 14).parse().unwrap_or(0);
        let surname = field(&r, 15);
        let given = field(&r, 16);
        let surname_yomi = field(&r, 17);
        let given_yomi = field(&r, 18);
        let text_url = Some(field(&r, 45)).filter(|s| !s.is_empty());

        let stem = text_url.and_then(extract_stem);

        if work_id == 773 && title.contains("こころ") {
            saw_kokoro = true;
        }

        works.push(AozoraWork {
            work_id,
            author_id,
            title,
            title_yomi,
            author: format!("{surname}{given}"),
            author_yomi: format!("{surname_yomi}{given_yomi}"),
            copyright_active,
            stem,
        });
    }

    if !saw_kokoro {
        return Err(Error::Invalid(
            "schema sanity check failed: work_id 773 / こころ not found. \
             Aozora may have changed CSV columns."
                .into(),
        ));
    }

    Ok(works)
}

fn field<'a>(r: &'a csv::StringRecord, idx: usize) -> &'a str {
    r.get(idx).unwrap_or("")
}

/// Extract the zip filename stem from a URL like
/// `https://www.aozora.gr.jp/cards/000081/files/45630_ruby_23610.zip`
/// → `Some("45630_ruby_23610")`.
pub fn extract_stem(url: &str) -> Option<String> {
    url.rsplit('/')
        .next()?
        .strip_suffix(".zip")
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_stem_ruby() {
        assert_eq!(
            extract_stem("https://www.aozora.gr.jp/cards/000081/files/45630_ruby_23610.zip"),
            Some("45630_ruby_23610".to_string())
        );
    }

    #[test]
    fn extract_stem_plain() {
        assert_eq!(
            extract_stem("https://www.aozora.gr.jp/cards/000148/files/789_14547.zip"),
            Some("789_14547".to_string())
        );
    }

    #[test]
    fn extract_stem_non_zip_url() {
        assert_eq!(
            extract_stem("https://www.aozora.gr.jp/cards/000148/files/789_14547.html"),
            None
        );
    }

    #[test]
    fn extract_stem_empty() {
        assert_eq!(extract_stem(""), None);
    }

    #[test]
    fn parse_index_against_fixture() {
        let dir = std::env::temp_dir().join(format!("jp-importer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv_path = dir.join("fixture.csv");
        std::fs::write(&csv_path, fixture_csv()).unwrap();

        let works = parse_index(&csv_path).expect("parse should succeed");
        let kokoro = works.iter().find(|w| w.work_id == 773).expect("kokoro");
        assert_eq!(kokoro.title, "こころ");
        assert_eq!(kokoro.author, "夏目漱石");
        assert!(!kokoro.copyright_active);
        assert_eq!(kokoro.stem.as_deref(), Some("773_14560"));

        let rashomon = works.iter().find(|w| w.work_id == 127).expect("rashomon");
        assert_eq!(rashomon.title, "羅生門");
        assert_eq!(rashomon.author_yomi, "あくたがわりゅうのすけ");
    }

    #[test]
    fn parse_index_fails_loudly_when_kokoro_missing() {
        let dir = std::env::temp_dir().join(format!("jp-importer-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv_path = dir.join("bad.csv");
        // Same shape but kokoro row missing.
        let header = fixture_csv().lines().next().unwrap().to_string();
        std::fs::write(&csv_path, header + "\n").unwrap();
        let err = parse_index(&csv_path).unwrap_err();
        assert!(format!("{err}").contains("schema sanity check"));
    }

    /// Header + two data rows, padded so target columns (0,1,2,4,15..19,45)
    /// land where parse_index expects.
    fn fixture_csv() -> String {
        // 51 columns total (so index 50 is reachable). Header content
        // doesn't matter for our parser, only column count.
        let mut header = String::new();
        for i in 0..51 {
            if i > 0 {
                header.push(',');
            }
            header.push_str(&format!("c{i}"));
        }

        let kokoro = data_row(
            773,
            "こころ",
            "こころ",
            "なし",
            148,
            "夏目",
            "漱石",
            "なつめ",
            "そうせき",
            "https://www.aozora.gr.jp/cards/000148/files/773_14560.zip",
        );
        let rashomon = data_row(
            127,
            "羅生門",
            "らしょうもん",
            "なし",
            879,
            "芥川",
            "龍之介",
            "あくたがわ",
            "りゅうのすけ",
            "https://www.aozora.gr.jp/cards/000879/files/127_15260.zip",
        );

        format!("{header}\n{kokoro}\n{rashomon}\n")
    }

    fn data_row(
        work_id: u32,
        title: &str,
        title_yomi: &str,
        copyright: &str,
        author_id: u32,
        surname: &str,
        given: &str,
        surname_yomi: &str,
        given_yomi: &str,
        text_url: &str,
    ) -> String {
        let mut cols = vec![String::new(); 51];
        cols[0] = work_id.to_string();
        cols[1] = title.into();
        cols[2] = title_yomi.into();
        cols[10] = copyright.into();
        cols[14] = author_id.to_string();
        cols[15] = surname.into();
        cols[16] = given.into();
        cols[17] = surname_yomi.into();
        cols[18] = given_yomi.into();
        cols[45] = text_url.into();
        cols.join(",")
    }
}
