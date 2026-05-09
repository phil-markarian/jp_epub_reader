# Appendix A — Aozora Bunko Chūki Format Reference

The Aozora Bunko `.txt` annotation language is called 注記 (chūki). This
appendix is a complete-enough reference for building a parser.

Authoritative source: https://www.aozora.gr.jp/annotation/

## File-level structure

```
{title}
{title-yomi (optional)}
{author}
[blank line]
[blank line]
-------------------------------------------------------
【テキスト中に現れる記号について】

《》：ルビ
（例）...

｜：ルビの付く文字列の始まりを特定する記号
（例）...

［＃］：入力者注　主に外字の説明や、傍点の位置の指定
（例）...

-------------------------------------------------------

{body}


底本：「{book title}」{publisher}
　　　{year}年{month}月{day}日{nth}刷発行
入力：{input person}
校正：{proofreader}
{date}作成
青空文庫作成ファイル：
このファイルは、インターネットの図書館、青空文庫（https://www.aozora.gr.jp/）で作られました。入力、校正、制作にあたったのは、ボランティアの皆さんです。
```

The `-------------------------------------------------------` line (55 hyphens)
is the structural separator. Everything between the second pair of separators
is either the markup legend (skip) or already the body, depending on file.
Convention: body is between the second occurrence of the separator and the
first line starting with `底本：`.

## Inline markup

### Ruby (furigana)

Two forms:

**Implicit** — base is the longest run of CJK chars immediately before `《`:
```
日本語《にほんご》
```
Base: `日本語`. Reading: `にほんご`.

**Explicit** — `｜` (FULLWIDTH VERTICAL LINE, U+FF5C) marks the start of
the base:
```
｜五月雨《さみだれ》
```
Without `｜`, the implicit rule would treat `雨` as the base (since `五月`
contains a non-CJK char if interpreted oddly, but here it's all CJK so
without `｜` the base would correctly be `五月雨`). The pipe is used when
the base contains non-CJK chars or when boundaries are ambiguous:

```
｜あの人《あのひと》     ← base contains hiragana
｜彼/彼女《かれ・かのじょ》  ← base contains a slash
```

**CJK ranges that count as ruby targets:**
- U+3400-U+4DBF (CJK Ext A)
- U+4E00-U+9FFF (CJK Unified)
- U+F900-U+FAFF (CJK Compat)
- U+20000-U+2FFFF (CJK Ext B+)

Some implementations also include kanji-like punctuation (々, 〆) — your
choice; AozoraEpub3 does.

### Chūki blocks `［＃...］`

`［` is U+FF3B, `＃` is U+FF03 (FULLWIDTH NUMBER SIGN), `］` is U+FF3D.
Many varieties; the content is plain text and ends at the first `］`.

```
［＃改ページ］                    page break
［＃ページの左右中央］             center on facing page
［＃改丁］                        chapter-break (recto)
［＃改見開き］                    spread break

［＃大見出し］章一［＃大見出し終わり］     large heading
［＃中見出し］節［＃中見出し終わり］       medium heading
［＃小見出し］...［＃小見出し終わり］      small heading

［＃ここから○字下げ］...［＃ここで字下げ終わり］
［＃ここから○字下げ、折り返して●字下げ］
［＃ここから○字下げ、●字詰め］
［＃地から○字上げ］              right-align with N-char gap

［＃「文字」に傍点］              boten emphasis on "文字"
［＃「文字」に傍線］              sideline on "文字"
［＃「文字」は太字］              bold "文字"
［＃「文字」は斜体］              italic "文字"
［＃「文字」に白ゴマ傍点］          white sesame dots
［＃「文字」に二重傍線］           double sideline

［＃挿絵（filename.png）入る］     image
［＃キャプション］...［＃キャプション終わり］
［＃縦中横］123［＃縦中横終わり］   horizontal-in-vertical

［＃割り注］...［＃割り注終わり］   warichu (split annotation)
［＃ルビ「reading」］...           ruby with explicit grouping
```

### Gaiji `※［＃...］`

Note the `※` (U+203B) prefix — distinguishes from regular chūki. The
content describes a character that wasn't in JIS X 0208 when Aozora was
designed. Several formats coexist:

```
※［＃「さんずい＋垂」、unicode6DB6］      U+6DB6 (溦)
※［＃U+845b］                            U+845B (葛)
※［＃U+845b-U+e0100］                    Variation Selector form
※［＃「さんずい＋垂」、U+6DB6、235-7］    U+6DB6 with JIS pos
※［＃「さんずい＋垂」、UCS6DB6、235-7］   alt notation
※［＃「てへん＋劣」、第3水準1-84-77］      JIS Level-3 position
※［＃「てへん＋劣」、第4水準2-84-77］      JIS Level-4 position
```

Resolution priority:
1. Explicit `unicodeXXXX` or `U+XXXX` hex — use it directly
2. `chuki_utf.txt` lookup by description (e.g., "さんずい＋垂")
3. `chuki_ivs.txt` lookup for IVS sequences
4. JIS position lookup if you have the table
5. Fallback: `〓` with a tooltip

Borrow `chuki_utf.txt` from AozoraEpub3 — it's a community-maintained
mapping with thousands of entries.

### Other inline markers

```
※                             literal asterisk (the gaiji marker only when
                              followed by ［＃)
～                             literal wave dash (often used as ellipsis)
　                             ideographic space (U+3000) — preserve in
                              vertical text
〳〴                           kunoji (curve repeat marks)
／＼                          horizontal repeat mark
／″＼                          dakuten variant repeat
```

## Block-level patterns

### Indent ranges

Two interacting ranges:

```
［＃ここから3字下げ］                  start: indent everything below 3 chars
本文 line 1
本文 line 2
［＃ここで字下げ終わり］                 end of indent block
```

Variant with hanging indent:
```
［＃ここから2字下げ、折り返して4字下げ］
First line at 2-char indent, wrapped lines at 4-char indent.
This is used for poetry, lists, etc.
［＃ここで字下げ終わり］
```

Variant with width constraint:
```
［＃ここから2字下げ、20字詰め］        2-char indent, 20-char column width
［＃ここで字下げ終わり］
```

Variant with single-line indent:
```
［＃天から3字下げ］この行のみ3字下げ
```

### Heading levels

```
［＃大見出し］...［＃大見出し終わり］      ~ <h1>
［＃中見出し］...［＃中見出し終わり］      ~ <h2>
［＃小見出し］...［＃小見出し終わり］      ~ <h3>
```

There's also "ゴシック" (gothic / sans-serif) variants which are
visual-only:
```
［＃ゴシック体］...［＃ゴシック体終わり］
```
Render as `<strong>` or with a CSS class, your call.

### Page/section breaks

```
［＃改ページ］          force page break (used between chapters)
［＃改丁］              recto-only break (preferred for chapter starts)
［＃改見開き］          break to facing-spread start
［＃ページの左右中央］   center content on the page (used for half-titles)
```

In EPUB 3 these are all most cleanly modeled as
`<hr epub:type="pagebreak"/>` plus appropriate CSS `page-break-before:
always` on the next block.

## Common gotchas

### Chūki applies to preceding text

For emphasis chūki, the markup REFERS BACK to text that already exists in
the token stream:

```
彼は誰だ？［＃「誰」に傍点］
```

When you hit `［＃「誰」に傍点］`, you need to find the most recent
`Inline::Text` containing `誰` and split it: `誰` becomes
`Inline::Emphasis`, the surrounding text stays `Inline::Text`.

This means your AST stage cannot be purely streaming — keep a small backlog
buffer or do two passes.

### Ruby boundary ambiguity

```
日本語《にほんご》
```
With implicit rules, base is `日本語` (3 CJK chars). Easy.

```
あの日本語《にほんご》
```
With implicit rules, base is `日本語` — the leading `あの` (hiragana) is
NOT included. Correct? Almost always yes. If the author wanted ruby on
`あの日本語`, they'd write `｜あの日本語《...》`.

```
の日本語《にほんご》
```
Same logic — base is `日本語`. If the leading `の` should be included, the
author writes `｜の日本語《...》`.

Edge case to watch: numerals and Latin chars. `a《えー》` — does `a` count
as a ruby target? AozoraEpub3 says no by default (only CJK). Your call;
match that for compatibility.

### Multiple chūki in sequence

```
彼［＃「彼」に傍点］は［＃改ページ］
```

Tokenize as: `彼`, `［＃「彼」に傍点］`, `は`, `［＃改ページ］`. The first
chūki modifies the preceding `彼` token; the second is a standalone block
marker.

### Nested or overlapping markup

Aozora's format generally avoids overlap, but consider:

```
［＃ここから2字下げ］
普通の段落。
［＃ここから3字下げ］
さらに字下げした段落。
［＃ここで字下げ終わり］
2字下げに戻る。
［＃ここで字下げ終わり］
```

You need a stack of indent levels. The outer 2-char indent resumes after
the inner 3-char indent ends.

### Encoding

Most files: Shift-JIS (specifically MS932 / CP932). Some recent files: UTF-8.
Detection: try SJIS first (`encoding_rs::SHIFT_JIS.decode(bytes)` — check
`had_errors`); if it errors heavily, fall back to UTF-8. AozoraEpub3 uses
the same heuristic.

### Line breaks

- Single newline = soft line break (still in same paragraph)
- Blank line = paragraph break

Different from prose convention but standard in Aozora. When rendering to
HTML: every soft break gets a `<br/>`; blank lines start a new `<p>`.

### Ideographic full-width spaces

`　` (U+3000) at the start of a line means "indent by one character." Don't
strip them silently — preserve them as text content. CSS `white-space:
pre-wrap` on `<p>` is the simplest way to render correctly.

## Test fixtures

Three works to verify each markup category in your converter:

| Test | Markup | File |
|---|---|---|
| Basic ruby (implicit + explicit) | `《》`, `｜` | Akutagawa "羅生門" (127) |
| Headings + 字下げ | `［＃大見出し］`, `［＃ここから○字下げ］` | Sōseki "こころ" (773) |
| Gaiji + emphasis | `※［＃...］`, `［＃「...」に傍点］` | Dazai "人間失格" (301) |

Snapshot the converter output for these three. CI fails if anything
changes.

## Reference files from AozoraEpub3

These ship with AozoraEpub3 and are useful to copy:

| File | What it is |
|---|---|
| `chuki_tag.txt` | Mapping of chūki text → HTML tag/class |
| `chuki_utf.txt` | Gaiji description → Unicode codepoint |
| `chuki_ivs.txt` | IVS (variation selector) gaiji mappings |
| `template/` | EPUB skeleton: container.xml, content.opf template, CSS |

Borrow them all. They represent years of accumulated knowledge about
edge cases. License is GPL-3 (same as AozoraEpub3 itself); embedding them
makes your project GPL-3.

## Further reading

- Aozora's official annotation reference (Japanese):
  https://www.aozora.gr.jp/annotation/
- AozoraEpub3 source notes:
  https://github.com/hmdev/AozoraEpub3 (original)
  https://aozoraepub3-jdk21.github.io/AozoraEpub3-JDK21/ (JDK21 fork)
- Aozora corpus generator (Python, useful as reference parser):
  https://github.com/borh/aozora-corpus-generator
