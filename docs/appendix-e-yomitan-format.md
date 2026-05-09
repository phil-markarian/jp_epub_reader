# Appendix E — Yomitan Dictionary Format

Phase 5 imports any Yomitan-format `.zip`. This appendix documents the
format so you can write the importer correctly.

References:
- Spec: https://github.com/yomidevs/yomitan/blob/master/docs/making-yomitan-dictionaries.md
- Schema files: https://github.com/yomidevs/yomitan/tree/master/ext/data/schemas
- Sample dictionaries: Jitendex, Meikyou, Shinmeikai (community)

## Zip structure

```
your-dictionary.zip
├─ index.json                  REQUIRED
├─ term_bank_1.json            term entries (paginated)
├─ term_bank_2.json
├─ ...
├─ term_meta_bank_1.json       frequency / pitch accent / IPA
├─ ...
├─ kanji_bank_1.json           kanji entries (KANJIDIC-style dicts)
├─ ...
├─ kanji_meta_bank_1.json      kanji frequency
├─ ...
├─ tag_bank_1.json             tag definitions
└─ ...
```

Files are paginated — `term_bank_1.json` through `term_bank_N.json` for
N up to several hundred for large dictionaries. Each contains an array
of entries; sizes vary but typically 1000-10000 entries per file.

## index.json

```json
{
  "title": "JMdict (English)",
  "format": 3,
  "revision": "JMdict.2024-04-30",
  "sequenced": true,
  "author": "scriptin",
  "url": "https://github.com/scriptin/jmdict-simplified",
  "description": "Optional",
  "attribution": "Optional",
  "frequencyMode": "rank-based",
  "tagMeta": { ... }
}
```

Required fields: `title`, `format`, `revision`. The `format` field is
the schema version — we support `3` (current). Older `1`/`2` need
upgrade paths or rejection.

## term_bank_*.json — term entries

Array of arrays. Each entry has 8 elements at fixed positions:

```json
[
  "食べる",                          // [0] expression (kanji form)
  "たべる",                          // [1] reading
  "v1 vt",                           // [2] definition tags (POS classes)
  "v1",                              // [3] deinflection rule classes
  100,                               // [4] popularity score
  [                                  // [5] glossary
    "to eat",
    "to live on (e.g. one's salary)"
  ],
  1234567,                           // [6] sequence number
  "common"                           // [7] term tags
]
```

### Field details

**[0] expression** — usually kanji form, sometimes kana-only. Indexed
for lookup.

**[1] reading** — kana reading. Indexed.

**[2] definition tags** — space-separated tags from the dictionary's
`tag_bank`. Examples: `n` (noun), `v1` (ichidan verb),
`vt` (transitive). Used in display, not lookup.

**[3] rules** — space-separated rule classes for deinflection. Critical
for filtering deinflection candidates. Standard classes:

| Class | Meaning |
|---|---|
| `v1` | ichidan verb (-ru) |
| `v5` | godan verb (any -u) |
| `vk` | irregular: kuru |
| `vs` | irregular: suru |
| `vs-i` | -suru class verbs |
| `adj-i` | i-adjective |
| `adj-ix` | yoi/ii special adjective |
| (empty) | not inflectable |

If a deinflection candidate's `to_rules` doesn't intersect this field,
the entry doesn't match. This is what prevents "見る" from being
mistakenly returned for "見て" lookup of a noun-form match.

**[4] score** — popularity rank. Higher = display first. Some dicts
encode frequency rank; some encode "use this gloss preferentially";
some leave it 0.

**[5] glossary** — array of glosses. Two formats:

*Plain strings:*
```json
["to eat", "to live on (e.g. one's salary)"]
```

*Structured content:*
```json
[
  "to eat",
  {
    "type": "structured-content",
    "content": [
      {
        "tag": "div",
        "content": [
          { "tag": "span", "lang": "en", "content": "alternate definition" }
        ]
      }
    ]
  }
]
```

Structured content is a tree of typed objects representing rich content:
text, links, images, lists, tables. See "Structured content" below.

**[6] sequence number** — JMdict-style entry ID for cross-reference
linking. Optional.

**[7] term tags** — space-separated tags applied to the term as a whole
(not per-sense). Examples: `common`, `dated`, `arch`. Different from
[2]; these are reading/word level, not gloss level.

## term_meta_bank_*.json — frequency, pitch, IPA

Three modes; each entry is `[expression, mode, data]`:

**Frequency:**
```json
["食べる", "freq", 1234]
```

Or with reading specified:
```json
["食べる", "freq", { "reading": "たべる", "frequency": 1234 }]
```

**Pitch accent:**
```json
["食べる", "pitch", {
    "reading": "たべる",
    "pitches": [{ "position": 2, "tags": ["common"] }]
}]
```

`position` is the kana index of the downstep (0 = heiban / no downstep).

**IPA:**
```json
["食べる", "ipa", { "reading": "たべる", "ipa": "[ta̠be̞ɾɯ̟ᵝ]" }]
```

## kanji_bank_*.json — kanji entries

Array of arrays. Each entry has 6 elements:

```json
[
  "食",                              // [0] character
  "ショク ジキ",                     // [1] onyomi (space-separated)
  "た た た く",                     // [2] kunyomi
  "common jouyou grade2",            // [3] tags (space-separated)
  ["food", "eat", "drink"],          // [4] meanings
  {                                  // [5] stats
    "grade": "2",
    "strokes": "9",
    "frequency": "468"
  }
]
```

## kanji_meta_bank_*.json — kanji frequency

Same shape as term_meta_bank for `freq` mode:

```json
["食", "freq", 468]
```

## tag_bank_*.json — tag definitions

Array of arrays. Each entry has 5 elements:

```json
[
  "v1",                              // [0] tag name
  "expression",                      // [1] category
  -3,                                // [2] sort key
  "Ichidan verb",                    // [3] description (full text)
  0                                  // [4] score
]
```

Categories include `expression`, `pos`, `name`, `dict`, `dialect`,
`form`, `frequency`, etc. Used to render tags with appropriate styling
in the popup.

## Structured content

The `glossary` field can contain either plain strings or structured
content objects. The structured content tree:

```typescript
type Content = string | ContentNode | ContentNode[];

interface ContentNode {
  tag: string;                // 'div', 'span', 'a', 'ul', 'li', 'br', 'img', etc.
  content?: Content;
  data?: { [key: string]: string };  // arbitrary metadata
  href?: string;              // for 'a'
  src?: string;               // for 'img'
  width?: number;             // for 'img'
  height?: number;            // for 'img'
  title?: string;             // accessibility
  lang?: string;              // language tag
  style?: { [key: string]: string };  // limited CSS
  fontStyle?: string;
  fontWeight?: string;
  fontSize?: string;
  textDecorationLine?: string;
  textDecorationStyle?: string;
  textDecorationColor?: string;
  borderColor?: string;
  borderStyle?: string;
  borderRadius?: string;
  borderWidth?: string;
  verticalAlign?: string;
  textAlign?: string;
  marginTop?: number;
  marginBottom?: number;
  marginLeft?: number;
  marginRight?: number;
  padding?: string;
  wordBreak?: string;
  whiteSpace?: string;
  collapsible?: boolean;      // expandable region
  collapsed?: boolean;        // initial state
}
```

Most dictionaries use a small subset: `div`, `span`, `a`, `ul`, `ol`,
`li`, `br`, `img`. Tables and complex layouts are rare.

For Phase 5 v1: render strings + the common tags. Punt the exotic ones
(collapsible, complex tables) — fall back to plain text extraction:

```rust
fn render_structured_content(node: &serde_json::Value) -> String {
    if let Some(s) = node.as_str() {
        return html_escape(s).to_string();
    }
    if let Some(obj) = node.as_object() {
        let tag = obj.get("tag").and_then(|v| v.as_str()).unwrap_or("span");
        let content = obj.get("content")
            .map(render_structured_content)
            .unwrap_or_default();
        match tag {
            "br" => "<br>".to_string(),
            "div" | "span" | "p" => format!("<{}>{}</{}>", tag, content, tag),
            "ul" | "ol" => format!("<{}>{}</{}>", tag, content, tag),
            "li" => format!("<li>{}</li>", content),
            "a" => {
                let href = obj.get("href")
                    .and_then(|v| v.as_str())
                    .unwrap_or("#");
                if href.starts_with("?query=") {
                    // internal cross-reference
                    format!("<a class='xref' data-href='{}'>{}</a>",
                            html_escape(href), content)
                } else if href.starts_with("http://") || href.starts_with("https://") {
                    format!("<a href='{}' rel='noopener'>{}</a>",
                            html_escape(href), content)
                } else {
                    content  // unknown protocol: drop the link
                }
            }
            _ => content,  // unknown tag: render content only
        }
    } else if let Some(arr) = node.as_array() {
        arr.iter().map(render_structured_content).collect::<Vec<_>>().join("")
    } else {
        String::new()
    }
}
```

**Important:** never output `<script>`, `<iframe>`, `<object>`,
`<embed>` from structured content even if a malicious dict claims
those tags. Use an allowlist, not a denylist.

## Common dictionaries to test against

When implementing Phase 5, test against:

| Dictionary | Source | Why |
|---|---|---|
| JMdict (English) | jmdict-simplified | Largest EN-JA, simplest format |
| Jitendex | jitendex.org | Most popular community EN-JA, structured content |
| Meikyou | community-converted | JA-JA monolingual, complex glosses |
| Shinmeikai | community-converted | JA-JA, different style |
| BCCWJ frequency | tatuylonen/wiktextract | term_meta only, no terms |
| NHK pitch accent | community | term_meta with pitch data |

Different dicts exercise different code paths. JMdict is plain strings;
Jitendex has heavy structured content; Meikyou has Japanese-only
glosses with embedded examples; frequency dicts have no terms at all,
just term_meta.

## What we don't support

- **Format versions 1 and 2** — too old, deprecated. Reject at import
  with a clear error pointing the user at upgrade tools.
- **`tag-meta` field on tag entries** — niche, rare; ignore safely.
- **Audio dictionaries** — Yomitan supports audio source plugins; we
  don't (yet). Glossary fields referencing audio render as plain text.
- **`pronunciation` mode in term_meta** — included in newer Yomitan
  format; treat as IPA fallback.

## Lookup performance

For a 200k-term JMdict + 50k-term Jitendex + frequency dict + pitch
dict (~250k rows total):

- Indexed lookup by expression: <5ms
- Lookup with up to 16 substring trials and deinflection: <50ms p95
- Term_meta join for frequency/pitch: <10ms additional

If you see slower than this, check that indexes are actually being used
(`EXPLAIN QUERY PLAN`). Common cause: `term_meta.expression` not
indexed, or join order suboptimal.
