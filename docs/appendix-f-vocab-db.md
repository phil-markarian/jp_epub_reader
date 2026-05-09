# Appendix F — Vocab DB Design

Design rationale for the schema and retention algorithm in Phase 4.

## Why four tables, not one or two?

A naïve design has one row per encounter:

```sql
CREATE TABLE encounter (
    headword TEXT, reading TEXT, sentence TEXT, source TEXT, time INTEGER, status TEXT
);
```

This breaks down quickly:

- **Status migrations are awkward.** If you've seen 食べる 200 times and
  finally decide to "track" it, you'd have to update 200 rows. Instead,
  status lives once on the word.
- **Retention destroys statistics.** If you trim old encounters, your
  count of "how many times I've seen 食べる" changes too — it shouldn't.
- **No source-level facts.** "What % of words in Kokoro do I know?"
  requires either denormalizing or expensive joins.
- **No first-seen / last-seen per source.** Useful question: "When did I
  first see 一寸 in Soseki vs Akutagawa?" The data should be one query
  away.

The four-table split solves these:

```
vocab            one row per word  ──── stable identity, status, count
  │
  ├── vocab_source   permanent (vocab × source) edges  ── source attribution
  │                                                        survives retention
  │
  └── encounter      sentence-level record   ── trimmable; provides
                                                 review context

source           one row per source        ── titles, opened-at, totals
```

## Why monotonic encounter_count?

`vocab.encounter_count` is incremented every time you see the word and
**never decremented**, even when the matching `encounter` rows get
trimmed.

Reason: stable lifetime stats matter for review heuristics. "Have I seen
this word 5 times or 500 times?" is a real distinction for deciding
whether to mark known. If trimming silently lowers the count, the
distinction disappears.

Tradeoff: the `encounter_count` and `COUNT(*) FROM encounter WHERE
vocab_id=?` will diverge after retention runs. They mean different
things — the first is "ever seen", the second is "in our retained
sample". Both are useful; the schema preserves both.

## Why vocab_source separate from encounter?

`vocab_source` records *that* you saw word W in source S, with a
running count. `encounter` records *which sentence* contained the
encounter.

If we only had `encounter`, retention would erase the fact that "I read
食べる in Kokoro 47 times" once those individual sentence rows get
trimmed. By splitting:

- `encounter` rows can be trimmed without losing source attribution
- `vocab_source.count_in_source` keeps incrementing
- `vocab_source.first_seen_at` / `last_seen_at` per source survive

This lets queries like "show me the first time I encountered 一寸 in
each Soseki work" work after retention has run, even if the actual
sentences are gone.

The cost: another row per (word, source) pair. For typical use (~50k
words × ~3 sources average), that's ~150k rows. Negligible at SQLite's
scale.

## Status state machine — why these five?

```
seen → tracked → learning → known
  ↘                  ↘
   ignored        ignored
```

The states answer specific UX questions:

| State | "What do I do with this word?" |
|---|---|
| seen | Don't surface it. I just looked it up to disambiguate. |
| tracked | Remind me of it. I want to remember it exists. |
| learning | Anki has it. Anki schedules; we don't duplicate. |
| known | Exclude from review surfaces. Don't surface again. |
| ignored | Don't surface. Could be a name, proper noun, garbage. |

Why not more states? We considered:

- **`forgotten`** (re-saw a "known" word, didn't recognize it): in
  practice, you'd just demote to `learning` or `tracked`. Anki handles
  forgetting via its scheduler.
- **`active learning` vs `mature learning`**: that's Anki's job.
- **Per-context status** ("known in literary, not in colloquial"):
  added complexity without clear benefit.

Why not fewer? Tried 3-state (seen/tracked/known) and 4-state
(seen/tracked/learning/known). Both broke down:

- 3-state: no place to record "the user clicked 'send to Anki'" — that
  needs to be distinct from `tracked` (because Anki is now responsible
  for review) and from `known` (because the user hasn't mastered it).
- 4-state: needed to handle "this is a name, drop it" without confusion.
  `ignored` solves it.

Five states is the minimum that makes review surfaces work cleanly.

## Retention algorithm — diversity-aware reservoir

Goal: keep the encounter table bounded (~hundreds of MB max) without
losing the data that makes review interesting.

**Naïve "keep most recent N":** for words you see frequently, you only
have recent encounters. The first time you saw 一寸 in Akutagawa is
gone. The encounter list becomes "the last 20 times in whatever you've
been reading lately" — not useful.

**Diversity-aware reservoir:** keep three categories:

1. **The first-ever encounter.** The "I learned this in X" moment.
2. **The most-recent encounter per source.** "The last time I saw it
   in Soseki" — preserves source attribution even if you've seen it
   500 times.
3. **The N most-recent overall.** Recent context for what's still
   fresh.

The intersection isn't double-counted — a word's first-ever encounter
is also its first-in-source for that source, and possibly within the
recent-N. The query uses `UNION` to dedupe.

For typical reading (50 sources, 200 high-frequency words, 1000s of
encounters each): post-retention size is bounded by
`words × (1 + sources + N)` ≈ `200 × (1 + 50 + 20) = 14,200` rows.
For lower-frequency words (most of vocab), the cap is rarely hit.

### Why background, not trigger?

Tempting design: `AFTER INSERT ON encounter` trigger that auto-trims
when count exceeds threshold. This was rejected:

- **Triggers run synchronously.** Every popup → encounter insert →
  trigger fires → potentially expensive DELETE → user-visible latency
  in the lookup popup
- **Hard to debug.** Trigger errors get swallowed. Logs are murky.
- **Hard to tune.** Want to bump keep_per_word from 20 to 30? Trigger
  rebuilds are awkward.

Instead: background task on a 5-minute interval. Encounter inserts
happen at full speed; trimming is amortized. Run also at startup (in
case the app was killed mid-week with retention pending) and on demand
when the user opens a vocabulary review view (their data should be
current).

### Tunable parameters

```rust
pub struct RetentionConfig {
    pub keep_per_word: i64,         // default 20
    pub interval_seconds: u64,      // default 300
    pub run_at_startup: bool,       // default true
    pub run_on_review_view: bool,   // default true
}
```

Power users can disable retention entirely (`keep_per_word: i64::MAX`)
and accept a larger DB. For a 1-year heavy reader: ~5GB without
retention vs ~50MB with default settings.

## FTS index on encounter sentences

Why FTS on sentences? Two queries it enables:

1. **"Find all encounters containing 食べる"** — useful when generating
   examples or remembering context
2. **"What sentence preceded the first time I saw 一寸?"** — narrative
   queries about your reading history

Trigram tokenization (FTS5 `tokenize="trigram"`) handles Japanese
without a CJK-aware tokenizer. Adequate for the use case; not as
precise as a true word-tokenized FTS but doesn't require setting up
mecab in SQLite.

Alternative: tokenize sentences with vibrato during ingest, store
tokens, run FTS on those. More accurate; more complex; can be added
later if trigram FTS proves insufficient.

## What we deliberately don't store

- **No reading position per source.** That's the reader's job.
- **No SRS scheduling data.** That's Anki's job.
- **No per-encounter user notes.** Notes are per-word
  (`vocab.notes`). Per-encounter notes felt like clutter that nobody
  would maintain.
- **No vector embeddings.** Considered and rejected: SQL handles
  kanji-sharing, lexical relations, co-occurrence, frequency cohorts
  without them. If we ever need semantic search, `sqlite-vec` can be
  added without restructuring.
- **No image attachments to encounters** (beyond the OCR-source case).
  Those would be in the source data, not the encounter.

## Common queries

A few useful queries for review surfaces and stats:

**Words you tracked but haven't seen in 30 days:**
```sql
SELECT v.* FROM vocab v
WHERE v.status = 'tracked'
  AND v.last_seen_at < (unixepoch() - 86400 * 30)
ORDER BY v.last_seen_at ASC
LIMIT 50;
```

**% known for a source:**
```sql
SELECT
  COUNT(CASE WHEN v.status = 'known' THEN 1 END) * 1.0 /
  COUNT(*) AS pct_known
FROM vocab_source vs
JOIN vocab v ON v.id = vs.vocab_id
WHERE vs.source_type = 'aozora' AND vs.source_ref = '773';
```

**Words appearing in multiple Soseki works:**
```sql
SELECT v.headword, COUNT(DISTINCT vs.source_ref) as soseki_works
FROM vocab v
JOIN vocab_source vs ON vs.vocab_id = v.id
JOIN source s ON s.type = vs.source_type AND s.ref = vs.source_ref
WHERE vs.source_type = 'aozora' AND s.author = '夏目漱石'
GROUP BY v.id
HAVING soseki_works >= 3
ORDER BY soseki_works DESC, v.encounter_count DESC;
```

**Most recent first encounter per source (post-retention safe):**
```sql
SELECT v.headword, vs.source_ref, vs.first_seen_at
FROM vocab v
JOIN vocab_source vs ON vs.vocab_id = v.id
ORDER BY vs.first_seen_at DESC
LIMIT 50;
```

This works even after the actual `encounter` rows for that pairing have
been trimmed — the timestamp is preserved on `vocab_source`.

**Heatmap data for weekly report:**
```sql
SELECT
  DATE(occurred_at, 'unixepoch', 'localtime') as day,
  COUNT(DISTINCT vocab_id) as unique_words,
  COUNT(*) as total_encounters
FROM encounter
WHERE occurred_at >= ?1
GROUP BY day
ORDER BY day;
```

## Schema migration checklist

When extending the schema in later phases:

1. New migration file: `crates/jp-vocab/src/migrations/00X_*.sql`
2. Append `include_str!()` to `MIGRATIONS` array
3. Run on a fresh DB and on a populated test DB; verify both succeed
4. Test idempotency: run migrations twice, second run is a no-op
5. Test resumability: kill mid-migration, restart; finishes cleanly
6. Document in JOURNAL with the schema rationale

Don't change existing migration files after they ship — write a new one
that fixes the problem. PRAGMA user_version becomes a one-way ratchet.

## Benchmarks (rough, M2 Mac)

For a database with ~50k vocab rows, ~200k vocab_source rows, ~500k
encounter rows (~6 months of moderately heavy use):

| Operation | Time |
|---|---|
| `record_encounter` (single) | ~0.5ms |
| `record_encounter` × 1000 (in transaction) | ~80ms |
| Lookup popup hit (full path through dict + encounter insert) | ~10-30ms |
| Retention pass (one batch, dirty words only) | ~100-300ms |
| FTS search "containing 食べる" | ~5ms |
| Weekly report query batch | ~50ms |
| `vocab_stats` for menu bar | ~3ms |

These are with WAL mode + indexes set up correctly. Without WAL, expect
3-10× slower writes.

## Backup & export

A full DB export is one file copy: `library.sqlite`. WAL mode means you
need the `-wal` and `-shm` files too if the app was running, but they
checkpoint automatically on clean shutdown.

For human-readable export (useful before destructive migrations):

```rust
pub fn export_vocab_to_jsonl(db: &VocabDb, out: &Path) -> Result<()> {
    // One JSONL line per vocab row, with embedded sources + recent encounters.
    // Useful for migration to a different tool, archival, or debugging.
}
```

Phase 4 should ship this export early. It's cheap insurance.
