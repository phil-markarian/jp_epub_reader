# Appendix B — Reader Options

Phase 3 needs an embedded reader. From Phase 5 onward, the reader must
capture click events to record encounters in the vocab DB. This rules
out external readers (Apple Books, ttu in browser) for vocab tracking.

Three viable embedded options. Pick one and commit; switching costs
real time.

## Comparison

| Feature | ttu (forked) | foliate-js | Roll your own |
|---|---|---|---|
| License | AGPL-3.0 | BSD-3-Clause | yours |
| Tategaki support | Native, polished | Native | DIY |
| Ruby support | Native | Native | DIY |
| Click capture | Via injected JS | Via injected JS | Native |
| Asset overhead | ~1MB JS bundle | ~150KB | ~0 |
| Build complexity | Vite + npm | None (pure JS modules) | Leptos-only |
| Time to v1 | 1-2 weekends | 1 weekend | 2-3 weekends |
| GPL compatibility | Yes (AGPL ⊃ GPL) | Yes | Yes |
| External Yomitan still works | Yes (separate workflow) | Yes | Yes |

## Option A — ttu (forked + embedded)

ttu is a Svelte SPA designed to run as a browser-based reader. Mature
EPUB rendering. Yomitan-friendly DOM structure. AGPL-3.

### Why it wins for v1

- **Best out-of-box rendering.** Tategaki, ruby, page navigation,
  reading progress — all polished.
- **DOM is Yomitan-shaped.** Even though we have our own dictionary, the
  same DOM patterns make hover/click handling straightforward.
- **Minimum custom UI work.** You inherit the reader chrome.

### What you have to do

1. Fork `https://github.com/ttu-ttu/ebook-reader`
2. Patch the entry point to accept a `loadBookFromBytes(bytes)` API
   (current entry expects a file dialog or URL)
3. Build the Svelte SPA with Vite
4. Bundle output (`dist/index.html` + assets) under
   `src-tauri/resources/ttu-reader/`
5. Tauri reader window loads `tauri://localhost/ttu-reader/index.html`
6. After load, frontend reads the EPUB from disk via
   `convertFileSrc` and calls the patched API
7. Inject lookup-popup JS via `initialization_script` on window create

The patch is a small change but does mean maintaining a fork. Pull
upstream changes ~quarterly; ttu development is moderate.

### Build pipeline

You need npm + Vite for ttu, despite "no npm" being a v1 goal. Two
options:

1. Build ttu in CI; commit `dist/` to your repo. Build hosts need npm
   one-time only; daily dev doesn't.
2. Build ttu via a `build.rs` that runs `npm ci && npm run build` if
   `dist/` is missing. Slower first build but more reproducible.

Either works. (1) is simpler if your CI is set up.

## Option B — foliate-js

Foliate is the GTK Linux ebook reader; foliate-js is its rendering core
extracted as a pure-JS module library. BSD-3 license. No build step
needed; ES modules import directly.

### Why it might win

- **No npm.** Pure ES modules + raw HTML+CSS. Drops `dist/` directly
  into resources/.
- **License flexibility.** BSD-3 vs AGPL — matters if you ever want
  closed-source distribution down the road.
- **Smaller bundle.** ~150KB vs ttu's ~1MB.
- **Active upstream.** John Factotum maintains it. Used in Foliate
  proper.

### What you have to do

1. Vendor `foliate-js` (a few JS files) into
   `src-tauri/resources/foliate-js/`
2. Build the reader chrome yourself in Leptos: header (title, position),
   navigation buttons, settings panel
3. Use foliate-js's `Book` and `Renderer` classes to load the EPUB and
   page through it
4. Inject lookup-popup JS the same way as Option A

### Tradeoff

You build the reader UI yourself. That's 1-2 days of work — toolbar,
TOC sidebar, position memory, font/theme controls. ttu gives you all of
that for free, but ties you to AGPL.

If your stack is already GPL throughout (yomitan + ttu + AozoraEpub3),
the AGPL constraint of ttu is moot. If you're hedging on license, take
foliate-js.

## Option C — Roll your own (Leptos + EPUB lib)

Render the EPUB yourself: parse the OPF, walk the spine, render each
chapter's XHTML inline in a Leptos component, paginate via CSS.

### Why it might win

- **Zero JS dependencies.** Pure Rust + Leptos.
- **Full control.** No fork to maintain.
- **No npm anywhere in the build.**

### What you have to do

1. EPUB parsing: `epub` or `rbook` crate, walk OPF
2. XHTML rendering: pass through to Leptos `inner_html` (carefully
   sanitized)
3. CSS for tategaki: `writing-mode: vertical-rl`
4. Ruby: native `<ruby>` works in WebKit
5. Pagination: CSS `column-width` + scroll snap, OR custom virtual
   pagination
6. Click capture: native Leptos events

### Why this is harder than it looks

- Ruby positioning across line breaks needs `ruby-position: over` and
  WebKit-specific quirks. Edge cases.
- Tategaki + columns + CSS scroll-snap is buggy in WebKit.
- Footnotes in Aozora EPUBs use cross-references (`<a epub:type="noteref">`)
  that you'd need to handle.
- Position memory across resize is non-trivial.

You'll spend at least a week getting the rendering to where ttu/foliate
are out of the box. Worth it only if license/build constraints make A
and B unworkable.

## Recommendation

**Start with Option A (ttu fork) if you don't care about AGPL.**
Fastest path to a working reader. Phase 3 done in a weekend.

**Switch to Option B (foliate-js) if you ever want closed-source.**
The migration is mechanical: same lookup-popup patterns, just a
different JS library underneath.

**Use Option C only if you have months and want to learn EPUB
internals.** Don't choose this for the wrong reasons.

## Click capture — common across all options

Whichever option you pick, the lookup-popup wiring looks the same. The
key requirements:

1. **Capture point**: shift-click + selection (matches Yomitan's UX
   conventions)
2. **Word boundary detection**: when the user clicks at offset N in a
   text node, the `lookup` command does substring extraction starting
   at N (the dictionary engine handles "find longest matching prefix")
3. **Sentence extraction**: walk text nodes around the click site to
   build the encounter sentence. Boundary characters: `。！？` plus
   newlines plus `<br>` plus `</p>` boundaries
4. **Source ref injection**: the reader window's
   `initialization_script` sets `window.__JP_SOURCE_REF__` to the
   current `source_id` (e.g., `aozora:773`)
5. **Popup positioning**: absolute positioning relative to viewport, not
   the clicked element (which scrolls)

```javascript
// Simplified pattern, applies to all three reader options
(function setupLookup() {
    const sourceRef = window.__JP_SOURCE_REF__;
    const sourceType = window.__JP_SOURCE_TYPE__;

    document.addEventListener('mouseup', async (e) => {
        if (!e.shiftKey) return;

        const sel = window.getSelection();
        if (!sel || sel.rangeCount === 0) return;

        const range = sel.getRangeAt(0);
        const startNode = range.startContainer;
        const offset = range.startOffset;

        if (startNode.nodeType !== Node.TEXT_NODE) return;

        const text = startNode.textContent;
        const sentence = extractSentence(startNode, offset);

        const result = await window.__TAURI__.core.invoke('lookup', {
            text: text.slice(offset),
            offset: 0,
            sourceType,
            sourceRef,
            sentence,
        });

        if (result) showPopup(result, e.clientX, e.clientY);
    });

    function extractSentence(node, offset) {
        const text = node.textContent;
        const before = text.slice(0, offset);
        const after = text.slice(offset);

        const beforeBoundary = Math.max(
            before.lastIndexOf('。'),
            before.lastIndexOf('！'),
            before.lastIndexOf('？'),
            before.lastIndexOf('\n')
        );

        const afterMatch = after.match(/[。！？\n]/);
        const afterBoundary = afterMatch ? offset + afterMatch.index + 1 : text.length;

        return text.slice(beforeBoundary + 1, afterBoundary).trim();
    }
})();
```

For multi-text-node sentences (which happen with Aozora EPUBs that wrap
ruby in spans), walk siblings via TreeWalker. Phase 5 problem; Phase 3
gets the simple case.

## License compatibility summary

Your stack:

- AozoraEpub3 (jar): GPL-3
- Yomitan format spec: GPL-3 inheritance via Yomitan
- Your code: GPL-3 (recommended)

Option A (ttu, AGPL-3): fine — you're already GPL
Option B (foliate-js, BSD-3): fine — BSD-3 ⊆ GPL-3
Option C (own code): fine — pick your own license

If you ever distribute closed-source, Option C only. AGPL would
contaminate; even with B you'd need to release the modified foliate-js
sources.
