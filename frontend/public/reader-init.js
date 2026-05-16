// Module-loaded helper that wires up foliate-js so the wasm reader can
// drop an EPUB blob in and get a rendered <foliate-view>. Loaded as a
// type=module script from index.html. Importing view.js registers the
// <foliate-view> custom element as a side effect.

import "./foliate-js/view.js";

const VERTICAL_DIR = "rtl"; // tategaki books page right-to-left
const READER_STATE_VERSION = 1;

const readerStateKey = (workId) => `jp-reader-state:v${READER_STATE_VERSION}:${workId}`;
// Global prefs (theme + flow direction) are shared across all books —
// these are presentation preferences for the reader chrome, not
// per-book state. Per-work state (lastLocation, etc.) still lives in
// the keyed `readerStateKey` entries above.
const GLOBAL_PREFS_KEY = `jp-reader-prefs:v${READER_STATE_VERSION}`;

const loadReaderState = (workId) => {
    try {
        const raw = window.localStorage?.getItem(readerStateKey(workId));
        if (!raw) return {};
        const parsed = JSON.parse(raw);
        return typeof parsed === "object" && parsed ? parsed : {};
    } catch {
        return {};
    }
};

const saveReaderState = (workId, patch) => {
    if (!workId) return;
    try {
        const next = { ...loadReaderState(workId), ...patch };
        window.localStorage?.setItem(readerStateKey(workId), JSON.stringify(next));
    } catch {
        // Best-effort only.
    }
};

const loadGlobalPrefs = () => {
    try {
        const raw = window.localStorage?.getItem(GLOBAL_PREFS_KEY);
        if (!raw) return {};
        const parsed = JSON.parse(raw);
        return typeof parsed === "object" && parsed ? parsed : {};
    } catch {
        return {};
    }
};

const saveGlobalPrefs = (patch) => {
    try {
        const next = { ...loadGlobalPrefs(), ...patch };
        window.localStorage?.setItem(GLOBAL_PREFS_KEY, JSON.stringify(next));
    } catch {
        // Best-effort only.
    }
};

/* ──────────────────────────────────────────────────────────────────────
 * Manual drag-region polyfill
 *
 * Tauri 2's auto-interception of `data-tauri-drag-region` isn't firing
 * in this build, so wire up mousedown → start_window_dragging
 * (Rust-side command) ourselves. Use the window label from the URL
 * query string so we don't depend on whichever frontend JS namespace
 * may or may not be exposed.
 * ────────────────────────────────────────────────────────────────── */
const currentWindowLabel = () => {
    try {
        const params = new URLSearchParams(window.location.search);
        const id = params.get("reader");
        return id ? `reader-${id}` : "main";
    } catch {
        return "main";
    }
};

const invokeTauri = async (cmd, args) => {
    const invoker =
        window.__TAURI__?.core?.invoke ?? window.__TAURI_INTERNALS__?.invoke;
    if (typeof invoker !== "function") {
        throw new Error("tauri invoke not available");
    }
    return invoker(cmd, args);
};

const installDragRegionHandler = () => {
    if (window.__JP_DRAG_REGION_INSTALLED) return;
    window.__JP_DRAG_REGION_INSTALLED = true;
    const isInteractive = (target) =>
        !!target.closest(
            "button, a, input, select, textarea, [contenteditable='true']",
        );

    // Use the WINDOW for capture-phase mousedown — this guarantees we
    // see the event before any Leptos-attached listener that might
    // stopPropagation. Capture phase also runs before wry's internal
    // drag-region detection, but since that detection seemingly does
    // nothing here, that's fine.
    window.addEventListener(
        "mousedown",
        (e) => {
            if (e.button !== 0) return;
            const target = e.target instanceof Element ? e.target : null;
            if (!target) return;
            if (isInteractive(target)) return;
            const region = target.closest("[data-tauri-drag-region]");
            if (!region) return;
            console.log("[reader-init][drag] mousedown on drag region", {
                target: target.tagName,
                region: region.tagName + "." + region.className,
                tauri: !!window.__TAURI__,
                internals: !!window.__TAURI_INTERNALS__,
            });
            const label = currentWindowLabel();
            invokeTauri("start_window_dragging", { label })
                .then(() => console.log("[reader-init][drag] startDragging ok"))
                .catch((err) =>
                    console.warn("[reader-init][drag] startDragging failed", err),
                );
        },
        true,
    );

    window.addEventListener(
        "dblclick",
        (e) => {
            const target = e.target instanceof Element ? e.target : null;
            if (!target) return;
            if (isInteractive(target)) return;
            const region = target.closest("[data-tauri-drag-region]");
            if (!region) return;
            const label = currentWindowLabel();
            invokeTauri("toggle_window_maximize", { label }).catch(() => {});
        },
        true,
    );
};
installDragRegionHandler();

/* ──────────────────────────────────────────────────────────────────────
 * Chapter resolution
 *
 * AozoraEpub3-converted EPUBs only put a fraction of their headings
 * into `toc.ncx` (Kokoro: just 中 + 下; 夢十夜 / 銀河鉄道の夜: an
 * empty navMap). But the converter *always* wraps every Aozora
 * 見出し chuki in inline markup:
 *
 *   ［＃大見出し］→ <div class="chap1">…</div>
 *   ［＃中見出し］→ <div class="chap2">…</div>
 *   ［＃小見出し］→ <div class="chap3">…</div>
 *
 * So we build a per-section cache of those inline markers at mount,
 * pre-walking every section via `book.sections[i].createDocument()`
 * so we have data even for sections the user never actually paginates
 * into. The resolver below first checks Foliate's tocItem (cheapest /
 * most precise when the EPUB has a real TOC), then falls back to the
 * inline cache, then to a book.toc walk by resolved section index.
 * Final composed label is "chap1 · chap2 · chap3" with empty levels
 * dropped.
 * ────────────────────────────────────────────────────────────────── */

const CHAPTER_CLASS_RE = /\bchap(\d+)\b/;
// Selector used both to extract and re-locate chapter elements. AozoraEpub3
// doesn't put DOM ids on the chap divs, so we navigate by "Nth match in
// this document" instead.
const CHAPTER_SELECTOR = '.chap1, .chap2, .chap3';

const extractChaptersFromDoc = (doc) => {
    if (!doc?.querySelectorAll) return [];
    const out = [];
    const nodes = doc.querySelectorAll(CHAPTER_SELECTOR);
    for (let i = 0; i < nodes.length; i++) {
        const el = nodes[i];
        const m = (el.className || "").match(CHAPTER_CLASS_RE);
        if (!m) continue;
        const level = parseInt(m[1], 10);
        if (!Number.isFinite(level)) continue;
        const label = (el.textContent || "").replace(/\s+/g, " ").trim();
        if (!label) continue;
        out.push({
            level,
            label,
            id: el.id || null,
            // Stable position in the section's chapter list. Used as
            // an anchor when navigating — the rendered iframe doc
            // matches the same CHAPTER_SELECTOR ordering.
            indexInSection: i,
        });
    }
    return out;
};

const setSectionChapters = (sectionIndex, list) => {
    if (typeof sectionIndex !== "number" || !window.__JP_READER) return;
    const map = window.__JP_READER._chaptersBySection;
    if (!(map instanceof Map)) return;
    map.set(sectionIndex, list);
};

/**
 * Pick the latest chapter in `list` whose DOM position is at or
 * before `range.startContainer`. Returns null when the range is
 * missing/incompatible (different document, etc.).
 *
 * AozoraEpub3 doesn't emit DOM ids on chap divs, so we locate each
 * cache entry by re-running CHAPTER_SELECTOR on the live doc and
 * indexing into the resulting NodeList. Order matches the order
 * captured at extraction time.
 */
const findChaptersAtOrBefore = (list, doc, range) => {
    if (!Array.isArray(list) || list.length === 0) return null;
    if (!range || !doc || !range.startContainer) return null;
    if (range.startContainer.ownerDocument !== doc
        && range.startContainer.getRootNode?.() !== doc) {
        return null;
    }
    const nodes = doc.querySelectorAll(CHAPTER_SELECTOR);
    let chap1 = null;
    let chap2 = null;
    let chap3 = null;
    let lastLevel = 0;
    for (const ch of list) {
        let el = ch.id ? doc.getElementById(ch.id) : null;
        if (!el && typeof ch.indexInSection === "number") {
            el = nodes[ch.indexInSection] ?? null;
        }
        if (!el) continue;
        let before;
        try {
            const cmp = el.compareDocumentPosition(range.startContainer);
            before = (cmp & Node.DOCUMENT_POSITION_FOLLOWING) !== 0
                || cmp === 0; // same node counts as "at"
        } catch {
            continue;
        }
        if (!before) break;
        // chapter is at or before visible position
        if (ch.level === 1) { chap1 = ch; chap2 = null; chap3 = null; }
        else if (ch.level === 2) { chap2 = ch; chap3 = null; }
        else if (ch.level === 3) { chap3 = ch; }
        lastLevel = ch.level;
    }
    return { chap1, chap2, chap3, lastLevel };
};

/**
 * Coarser resolution by section index alone — used as a fallback
 * when we don't have a DOM range or the range comparison failed,
 * and by the live-resolve-at-display path for old bookmarks.
 */
const sectionChapterDefaults = (sectionIndex) => {
    const map = window.__JP_READER?._chaptersBySection;
    if (!(map instanceof Map)) return null;
    if (typeof sectionIndex !== "number") return null;

    // First, see if this section has its own chap1. Walk backward
    // through sections if not; the first chap1 we find is the
    // currently-active book part.
    let chap1 = null;
    for (let i = sectionIndex; i >= 0 && chap1 == null; i--) {
        const list = map.get(i) || [];
        const found = list.find(c => c.level === 1);
        if (found) chap1 = found;
    }
    // chap2 is the first chap2 of the section (we're at the start of
    // the section by this lookup).
    const list = map.get(sectionIndex) || [];
    const chap2 = list.find(c => c.level === 2) || null;
    const chap3 = list.find(c => c.level === 3) || null;
    return { chap1, chap2, chap3, lastLevel: chap3?.level ?? chap2?.level ?? chap1?.level ?? 0 };
};

const composeChapterLabel = (parts) => {
    if (!parts) return null;
    const pieces = [parts.chap1, parts.chap2, parts.chap3]
        .filter(p => p && typeof p.label === "string" && p.label.length > 0)
        .map(p => p.label);
    return pieces.length ? pieces.join(" · ") : null;
};

const tocWalkLabel = (view, currentIndex) => {
    const book = view?.book;
    if (!book?.toc || typeof book.resolveHref !== "function") return null;
    if (typeof currentIndex !== "number") return null;
    let bestLabel = null;
    let bestIndex = -1;
    const visit = (items) => {
        if (!Array.isArray(items)) return;
        for (const item of items) {
            if (typeof item?.href === "string") {
                try {
                    const r = book.resolveHref(item.href);
                    if (
                        r && typeof r.index === "number"
                        && r.index <= currentIndex
                        && r.index > bestIndex
                        && typeof item.label === "string"
                        && item.label.trim().length > 0
                    ) {
                        bestLabel = item.label.trim();
                        bestIndex = r.index;
                    }
                } catch {
                    // ignore unresolvable hrefs
                }
            }
            if (item?.subitems?.length) visit(item.subitems);
        }
    };
    visit(book.toc);
    return bestLabel;
};

/**
 * Best-effort chapter label for the user's current location.
 *
 * Strategy: build a {chap1, chap2, chap3} triple from the inline
 * cache (which has the finer chap2/chap3 detail) and let Foliate's
 * tocItem override chap1 when present — TOC labels tend to be
 * cleaner ("下 先生と遺書") than the inline div the converter emits.
 * Falls back to tocItem alone, then to the book.toc href walk.
 */
const inferChapterLabel = (view, detail) => {
    const tocLabel = typeof detail?.tocItem?.label === "string"
        ? detail.tocItem.label.trim()
        : "";
    const currentIndex = detail?.section?.current;
    const map = window.__JP_READER?._chaptersBySection;

    if (map instanceof Map && typeof currentIndex === "number") {
        const list = map.get(currentIndex) || [];
        const range = detail?.range;
        const doc = range?.startContainer?.ownerDocument ?? null;
        let parts = list.length ? findChaptersAtOrBefore(list, doc, range) : null;
        // When the range walk yielded no chap1 (e.g. range missing or
        // current section has no chap1 marker), fall through to the
        // section defaults so we still get a part label.
        if (!parts || parts.chap1 == null) {
            const defaults = sectionChapterDefaults(currentIndex);
            if (defaults) {
                if (parts) {
                    parts = {
                        chap1: parts.chap1 ?? defaults.chap1,
                        chap2: parts.chap2 ?? defaults.chap2,
                        chap3: parts.chap3 ?? defaults.chap3,
                        lastLevel: parts.lastLevel || defaults.lastLevel,
                    };
                } else {
                    parts = defaults;
                }
            }
        }
        const composed = composeChapterLabelWithToc(parts, tocLabel);
        if (composed) return composed;
    }

    if (tocLabel) return tocLabel;
    return tocWalkLabel(view, currentIndex);
};

const composeChapterLabelWithToc = (parts, tocLabel) => {
    const chap1Label = (parts?.chap1 && parts.chap1.label) || tocLabel || null;
    const chap2Label = parts?.chap2?.label || null;
    const chap3Label = parts?.chap3?.label || null;
    const pieces = [chap1Label, chap2Label, chap3Label].filter(
        (s) => typeof s === "string" && s.length > 0,
    );
    return pieces.length ? pieces.join(" · ") : null;
};

/**
 * Live, BCR-driven chapter detection. Foliate's relocate event is
 * debounced to ~250ms in scrolled flow, so the highlighted row in
 * the chapter drawer lags behind the scroll position. This walks the
 * cached chapter list for the currently-visible section and uses the
 * live `getBoundingClientRect()` of each chapter element to pick
 * whichever one is at or before the viewport's leading edge — which
 * we can recompute on every rAF tick without waiting for relocate.
 *
 * Returns null when no chapter cache exists or no section is loaded.
 */
const computeLiveChapterFlatIndex = (view) => {
    const renderer = view?.renderer;
    if (!renderer || typeof renderer.getContents !== "function") return null;
    const contents = renderer.getContents();
    if (!Array.isArray(contents) || contents.length === 0) return null;
    const { doc, index: sectionIndex } = contents[0];
    if (!doc || typeof sectionIndex !== "number") return null;

    const flat = window.__JP_READER?.getChapterList?.() || [];
    if (!flat.length) return null;

    // Foliate scrolls an outer #container in the paginator's CLOSED
    // shadow DOM, so renderer.shadowRoot is null from outside — but
    // we can still reach the iframe element via doc.defaultView's
    // frameElement back-reference, which crosses the shadow boundary
    // because the iframe lives in its own window context. Element
    // BCRs from inside the iframe are iframe-internal (never move
    // under scroll); combining them with the iframe element's
    // parent-screen BCR (which DOES shift as the outer container
    // scrolls) gives us each chapter's true on-screen position.
    const iframeEl = doc.defaultView?.frameElement || null;
    if (!iframeEl) return null;
    const iframeBcr = iframeEl.getBoundingClientRect();
    // The paginator host element fills the visible viewport (its
    // BCR equals the closed #container's BCR), so we use it as the
    // visible-region reference frame.
    const containerBcr = renderer.getBoundingClientRect();

    const cs = doc.defaultView?.getComputedStyle?.(doc.documentElement);
    const wm = (cs?.writingMode || "horizontal-tb").toLowerCase();
    const isVerticalRL = wm.startsWith("vertical-rl");
    const isVerticalLR = wm.startsWith("vertical-lr");

    // "Has this chapter heading's leading edge crossed the visible
    // region's leading edge in screen coordinates?"
    // - vertical-rl: text flows right→left, so a chapter is entered
    //   when its screen-right edge has reached or passed the
    //   container's right edge.
    // - vertical-lr: opposite, entered when its left passes the
    //   container's left.
    // - horizontal-tb: entered when its top has passed the container
    //   top (scrolled) or left (paginated ltr).
    const passed = (el) => {
        const r = el.getBoundingClientRect();
        const screenLeft = iframeBcr.left + r.left;
        const screenRight = iframeBcr.left + r.right;
        const screenTop = iframeBcr.top + r.top;
        if (isVerticalRL) return screenRight <= containerBcr.right + 1;
        if (isVerticalLR) return screenLeft <= containerBcr.left + 1;
        return screenTop <= containerBcr.top + 1;
    };

    const docNodes = doc.querySelectorAll(CHAPTER_SELECTOR);
    let posInSection = -1;
    for (let i = 0; i < docNodes.length; i++) {
        if (passed(docNodes[i])) posInSection = i;
        else break;
    }

    // No chapter element in the current section has been entered —
    // we're either above the first chap1/chap2 in this section, or
    // this section has no markers. Fall back to the latest chapter
    // from prior sections.
    if (posInSection < 0) {
        let cand = -1;
        for (let i = 0; i < flat.length; i++) {
            if (flat[i].sectionIndex < sectionIndex) cand = i;
            else break;
        }
        return cand >= 0 ? cand : null;
    }

    // Walk flat list to find the position-th entry in this section.
    let nthInSection = 0;
    for (let i = 0; i < flat.length; i++) {
        const f = flat[i];
        if (f.sectionIndex !== sectionIndex) continue;
        if (nthInSection === posInSection) return i;
        nthInSection++;
    }
    return null;
};

/**
 * Install a chapter-change tracker on the live view. Listens to the
 * renderer's undebounced 'scroll' event (and relocate as a backstop),
 * rAF-throttles, and fires the registered callback whenever the
 * computed flat chapter index differs from the last reported one.
 *
 * Idempotent — re-mounting a view with the same global state will
 * just reset the lastIdx tracker; the listeners are bound to the
 * renderer, which is replaced on each mount().
 */
const installChapterTracker = (view) => {
    const renderer = view?.renderer;
    if (!renderer) return;

    let pending = false;
    const fire = () => {
        if (pending) return;
        pending = true;
        requestAnimationFrame(() => {
            pending = false;
            const idx = computeLiveChapterFlatIndex(view);
            const last = window.__JP_READER._lastChapterIdx ?? null;
            if (idx === last) return;
            window.__JP_READER._lastChapterIdx = idx;
            const cb = window.__JP_READER._chapterChangeCb;
            if (typeof cb === "function") {
                try { cb(idx); }
                catch (e) { console.warn("[reader-init] chapter cb threw", e); }
            }
        });
    };

    renderer.addEventListener("scroll", fire);
    view.addEventListener("relocate", fire);
    view.addEventListener("load", fire);

    // Prime once after install so the initial chapter highlights
    // even without any user interaction.
    fire();
};

const savedLocationFromRelocate = (detail) => {
    if (typeof detail?.cfi === "string" && detail.cfi.length > 0) {
        return detail.cfi;
    }
    if (typeof detail?.fraction === "number") {
        return { fraction: detail.fraction };
    }
    return null;
};

/** Themed CSS injected into every section iframe. */
const themeCSS = (theme) => {
    if (theme === "dark") {
        return `
            html, body, * { color: #e8e8e8 !important; background: transparent !important; }
            html, body { background: #1a1a1a !important; }
            a:link { color: #8ec5ff !important; }
        `;
    }
    if (theme === "sepia") {
        return `
            html, body, * { color: #3b2f1c !important; background: transparent !important; }
            html, body { background: #f4ecd8 !important; }
            a:link { color: #6b3d00 !important; }
        `;
    }
    // light (default)
    return `
        html, body, * { color: #111 !important; background: transparent !important; }
        html, body { background: #ffffff !important; }
        a:link { color: #1a4fb0 !important; }
    `;
};

const buildCSS = (theme, fontScale, lineHeight) => {
    return themeCSS(theme) + `
        html { font-size: ${(fontScale * 100).toFixed(0)}% !important; }
        p, li, blockquote, dd, div { line-height: ${lineHeight} !important; }
    `;
};

const reapplyStyles = () => {
    const view = window.__JP_READER._lastView;
    const renderer = view?.renderer;
    if (!renderer) return;
    const theme = window.__JP_READER._theme || "light";
    const scale = window.__JP_READER._fontScale ?? 1.0;
    const lh = window.__JP_READER._lineHeight ?? 1.7;
    renderer.setStyles?.(buildCSS(theme, scale, lh));
};

/** Background colors used for the chrome around the rendered page. */
const themeBg = (theme) => {
    if (theme === "dark") return "#1a1a1a";
    if (theme === "sepia") return "#f4ecd8";
    return "#ffffff";
};

const themeFg = (theme) => {
    if (theme === "dark") return "#e8e8e8";
    if (theme === "sepia") return "#3b2f1c";
    return "#111111";
};

const applyChromeColors = (theme) => {
    const root = document.documentElement;
    root.style.setProperty("--reader-bg", themeBg(theme));
    root.style.setProperty("--reader-fg", themeFg(theme));
};

/**
 * Foliate's paginator caps each column with max-inline-size /
 * max-block-size. For tategaki the inline axis is *vertical*, so
 * max-inline-size controls column height. We size paginated columns
 * to a comfortable height and width so the page actually fills the
 * window, and drop the caps entirely in scrolled mode so the stream
 * grows to fit.
 */
const applyFlowSizing = (renderer, flow) => {
    if (flow === "scrolled") {
        renderer.removeAttribute("max-inline-size");
        renderer.removeAttribute("max-block-size");
        renderer.removeAttribute("max-column-count");
    } else {
        // Tategaki vertical-rl: inline = vertical (column height),
        // block = horizontal (column width). Single column so wheel/
        // arrow keys advance one page at a time instead of skipping
        // past two columns simultaneously.
        renderer.setAttribute("max-inline-size", "900");
        renderer.setAttribute("max-block-size", "900");
        renderer.setAttribute("max-column-count", "1");
    }
};

// Wheel events fire inside the iframe that holds the rendered section
// content; they don't bubble out to our top-level listeners. Attach
// the same handler to every section iframe as it loads so the wheel
// works no matter where the cursor sits.
//
// Smooth horizontal-scroll wheel handling for scrolled flow now lives
// inside Foliate's paginator (see foliate-js/paginator.js
// #feedWheel/#setLogicalScroll). The app layer only maps the gesture
// to a logical delta and forwards it via view.feedWheel — keeping the
// reader off of renderer shadow-root internals.

// Pixels-per-wheel-event multiplier. Trackpad deltas are small; this
// scales them into a meaningful per-event scroll amount before they
// land in the paginator's queue.
const SCROLLED_WHEEL_GAIN = 1.6;
// Keyboard arrow nudge for scrolled mode (passed as distance to
// view.next/prev). Foliate animates these over ~300ms.
const SCROLLED_KEY_STEP = 220;

const isScrolledFlow = () => (window.__JP_READER._flow || "paginated") === "scrolled";
const isVerticalWriting = () => window.__JP_READER._verticalWriting !== false;

const captureLayoutFromDoc = (doc) => {
    if (!doc?.defaultView) return;
    const { writingMode } = doc.defaultView.getComputedStyle(doc.body);
    window.__JP_READER._verticalWriting =
        writingMode === "vertical-rl" || writingMode === "vertical-lr";
};

const wheelDeltaForScrolledMode = (deltaX, deltaY) => {
    // Vertical-writing scrolled mode reads horizontally. Map gesture
    // to advance intent: scroll-down or swipe-left = advance forward
    // (positive); scroll-up or swipe-right = retreat backward.
    if (isVerticalWriting()) {
        return (deltaY - deltaX) * SCROLLED_WHEEL_GAIN;
    }
    // Horizontal-writing fallback: dominant axis wins.
    const primary = Math.abs(deltaY) >= Math.abs(deltaX) ? deltaY : deltaX;
    return primary * SCROLLED_WHEEL_GAIN;
};

const feedScrolledWheel = (delta) => {
    if (!delta) return;
    const view = window.__JP_READER._lastView;
    view?.feedWheel?.(delta);
};

const scrollScrolledByKey = (direction) => {
    const view = window.__JP_READER._lastView;
    if (!view) return false;
    const distance = SCROLLED_KEY_STEP;
    if (direction > 0) view.next?.(distance);
    else view.prev?.(distance);
    return true;
};

const smoothScrollLeft = () => scrollScrolledByKey(1);
const smoothScrollRight = () => scrollScrolledByKey(-1);

/* In paginated mode, one wheel gesture should generally mean one page
   turn. Trackpad inertia can keep emitting events long after the user
   stops touching the pad, so we treat nearby same-direction events as
   one gesture burst and allow at most one page flip within that burst.
   A new burst starts only after a brief quiet gap or a direction
   change, which forces a small stop between page turns and prevents
   inertia from spilling into extra pages. */
let paginatedCarry = 0;
let paginatedBurstDirection = 0;
let paginatedLastEventAt = 0;
let paginatedBurstTurned = false;
const PAGE_TURN_THRESHOLD = 110;
const PAGE_BURST_GAP_MS = 30;
const WHEEL_MIN_DELTA = 1;

const onWheelInner = (ev) => {
    if (isScrolledFlow()) {
        ev.preventDefault();
        feedScrolledWheel(wheelDeltaForScrolledMode(ev.deltaX, ev.deltaY));
        return;
    }

    // Paginated: treat the wheel stream as repeated "next/prev page"
    // intent instead of requiring the gesture to fully quiet down.
    ev.preventDefault();
    const dy = ev.deltaY + ev.deltaX;
    if (Math.abs(dy) < WHEEL_MIN_DELTA) return;

    const view = window.__JP_READER._lastView;
    if (!view) return;

    const now = performance.now();
    const dir = Math.sign(dy);
    const quietGap = now - paginatedLastEventAt;
    const shouldStartNewBurst =
        paginatedBurstDirection === 0 ||
        dir !== paginatedBurstDirection ||
        quietGap > PAGE_BURST_GAP_MS;

    if (shouldStartNewBurst) {
        paginatedCarry = 0;
        paginatedBurstDirection = dir;
        paginatedBurstTurned = false;
    }

    paginatedLastEventAt = now;
    if (paginatedBurstTurned) return;

    paginatedCarry += dy;
    if (Math.abs(paginatedCarry) < PAGE_TURN_THRESHOLD) return;

    paginatedBurstTurned = true;
    if (dir > 0) view.next?.();
    else view.prev?.();
};

const performReaderAction = (action, key) => {
    const view = window.__JP_READER._lastView;
    if (!view) return false;

    if (isScrolledFlow()) {
        // In scrolled mode, arrows use section-aware distance
        // navigation, while wheel input owns continuous container scroll.
        if (["ArrowLeft", "ArrowDown", "h", "j"].includes(key)) {
            smoothScrollLeft();
            return true;
        }
        if (["ArrowRight", "ArrowUp", "l", "k"].includes(key)) {
            smoothScrollRight();
            return true;
        }
    }

    if (action === "next") {
        view.next?.();
        return true;
    }
    if (action === "prev") {
        view.prev?.();
        return true;
    }
    return false;
};

const onKeyNavInner = (ev) => {
    if (ev.metaKey || ev.ctrlKey || ev.altKey) return;

    if (ev.key === "ArrowDown" || ev.key === "ArrowRight") {
        ev.preventDefault();
        performReaderAction("next", ev.key);
    } else if (ev.key === "ArrowUp" || ev.key === "ArrowLeft") {
        ev.preventDefault();
        performReaderAction("prev", ev.key);
    }
};

// Some iframes are created before the load event we hook into; sweep
// for them once after mount so we don't miss the very first section.
const attachAllIframes = (root) => {
    if (!root) return;
    const visit = (node) => {
        if (!node) return;
        if (node.tagName === "IFRAME") {
            try {
                if (node.contentDocument) {
                    attachWheel(node.contentDocument);
                    attachKeyNav(node.contentDocument);
                }
                if (node.contentWindow) {
                    attachWheel(node.contentWindow);
                    attachKeyNav(node.contentWindow);
                }
            } catch (e) {
                // Cross-origin iframe: skip silently.
            }
        }
        if (node.shadowRoot) {
            node.shadowRoot.childNodes.forEach(visit);
        }
        node.childNodes?.forEach?.(visit);
    };
    visit(root);
};

const attachWheel = (target) => {
    if (!target || target.__jpWheelAttached) return;
    target.__jpWheelAttached = true;
    target.addEventListener("wheel", onWheelInner, { passive: false });
};

const attachKeyNav = (target) => {
    if (!target || target.__jpKeyNavAttached) return;
    target.__jpKeyNavAttached = true;
    target.addEventListener("keydown", onKeyNavInner, { capture: true });
};

/* ──────────────────────────────────────────────────────────────────────
 * Shift-hover dictionary lookup
 *
 * Mirrors Yomitan's scanner pattern: when the user holds Shift and
 * moves the cursor over text inside the foliate iframe, we resolve the
 * text node + offset under the cursor, slice up to LOOKUP_MAX_SCAN_LEN
 * characters of forward text, and ask the backend's dict_lookup
 * command for ranked hits. The result + click position are written to
 * window.__JP_LOOKUP_RESULT, which the Leptos popup component
 * subscribes to via a polling Effect (cheap — same callback pattern as
 * the chapter tracker).
 *
 * Throttled to one lookup per 80ms so a sweep across a sentence
 * doesn't fire dozens of IPC roundtrips.
 * ────────────────────────────────────────────────────────────────── */
const LOOKUP_MAX_SCAN_LEN = 16;
const LOOKUP_THROTTLE_MS = 80;

const attachLookupHover = (doc) => {
    try {
        if (!doc || doc.__jpLookupAttached) return;
        doc.__jpLookupAttached = true;
        let last = 0;
        doc.addEventListener("mousemove", (ev) => {
            if (!ev.shiftKey) return;
            const now = Date.now();
            if (now - last < LOOKUP_THROTTLE_MS) return;
            last = now;
            safeTriggerLookup(doc, ev.clientX, ev.clientY);
        });
        // Also fire on Shift-click for trackpads that suppress
        // mousemove until click.
        doc.addEventListener("click", (ev) => {
            if (!ev.shiftKey) return;
            safeTriggerLookup(doc, ev.clientX, ev.clientY);
        });
        console.log("[lookup] hover attached on iframe doc");
    } catch (e) {
        console.warn("[lookup] attachLookupHover failed", e);
    }
};

const safeTriggerLookup = (doc, x, y) => {
    try {
        triggerLookupAt(doc, x, y);
    } catch (e) {
        console.warn("[lookup] triggerLookupAt threw", e);
    }
};

const triggerLookupAt = (doc, x, y) => {
    const range = caretRangeAt(doc, x, y);
    if (!range) return;
    const text = extractForwardText(range, LOOKUP_MAX_SCAN_LEN);
    if (!text) return;
    // Re-fire of the same string is harmless but wastes IPC; skip.
    if (window.__JP_LOOKUP_LAST_TEXT === text) {
        publishPosition(x, y, doc);
        return;
    }
    window.__JP_LOOKUP_LAST_TEXT = text;
    publishPosition(x, y, doc);
    const invoker =
        window.__TAURI__?.core?.invoke ?? window.__TAURI_INTERNALS__?.invoke;
    if (typeof invoker !== "function") return;
    invoker("dict_lookup", { text, maxScanLen: LOOKUP_MAX_SCAN_LEN })
        .then((hits) => {
            window.__JP_LOOKUP_RESULT = {
                text,
                hits: Array.isArray(hits) ? hits : [],
                at: Date.now(),
            };
        })
        .catch((e) => console.warn("[lookup] failed", e));
};

/**
 * Best-effort wrapper around the browser's "caret position from
 * pixel coords" API. Different engines call it different things
 * and WKWebView only exposes the older `caretRangeFromPoint`.
 */
const caretRangeAt = (doc, x, y) => {
    if (typeof doc.caretRangeFromPoint === "function") {
        return doc.caretRangeFromPoint(x, y);
    }
    if (typeof doc.caretPositionFromPoint === "function") {
        const p = doc.caretPositionFromPoint(x, y);
        if (!p) return null;
        const r = doc.createRange();
        r.setStart(p.offsetNode, p.offset);
        r.setEnd(p.offsetNode, p.offset);
        return r;
    }
    return null;
};

/**
 * Concatenate up to `maxChars` characters of text starting at the
 * range's startContainer + startOffset, walking forward through
 * sibling text nodes when needed. Skips whitespace-only nodes and
 * `<rt>` ruby annotations (we want the base text, not the
 * pronunciation gloss).
 */
const extractForwardText = (range, maxChars) => {
    const node = range.startContainer;
    if (!node) return "";
    const root = node.ownerDocument?.body;
    if (!root) return "";

    let collected = "";
    if (node.nodeType === Node.TEXT_NODE) {
        collected = node.data.slice(range.startOffset);
    }

    // Walk forward through the document collecting more text until
    // we hit the cap. Use a TreeWalker rooted at <body> for speed.
    const walker = node.ownerDocument.createTreeWalker(
        root,
        NodeFilter.SHOW_TEXT,
        {
            acceptNode: (n) => {
                // Skip ruby annotations and CSS-hidden runs.
                const parent = n.parentElement;
                if (parent) {
                    const tag = parent.tagName?.toLowerCase();
                    if (tag === "rt" || tag === "rp") return NodeFilter.FILTER_REJECT;
                    if (tag === "script" || tag === "style") return NodeFilter.FILTER_REJECT;
                }
                return NodeFilter.FILTER_ACCEPT;
            },
        },
    );
    // Position the walker at our start node.
    walker.currentNode = node;
    // Move past the current node — we've already taken its tail above.
    let cur = walker.nextNode();
    while (cur && Array.from(collected).length < maxChars) {
        collected += cur.data;
        cur = walker.nextNode();
    }
    // Trim to maxChars characters (codepoint-aware).
    const chars = Array.from(collected);
    return chars.slice(0, maxChars).join("");
};

const publishPosition = (x, y, doc) => {
    // x/y are iframe-local coords. Translate to viewport coords by
    // adding the iframe element's bounding rect, then up to the
    // outermost window (in case there's nesting).
    const win = doc.defaultView;
    const frame = win?.frameElement;
    let outerX = x;
    let outerY = y;
    if (frame) {
        const r = frame.getBoundingClientRect();
        outerX += r.left;
        outerY += r.top;
    }
    window.__JP_LOOKUP_POSITION = { x: outerX, y: outerY, at: Date.now() };
};

window.__JP_READER = {
    /**
     * @param {HTMLElement} container - element to append the view into
     * @param {Blob | File} blob - the EPUB
     * @param {(detail: any) => void} onRelocate - called on every page change
     * @param {(detail: any) => void} onLoad - called when a section finishes loading
     * @returns {Promise<{ view: HTMLElement }>}
     */
    async mount(container, blob, workId, onRelocate, onLoad) {
        try {
            // Reset container so re-entries don't stack views.
            container.replaceChildren();
            window.__JP_READER._workId = workId;
            // Fresh chapter cache per mount — different works can have
            // identical section_index values, so we never want to carry
            // an old book's cache over.
            window.__JP_READER._chaptersBySection = new Map();
            const saved = loadReaderState(workId);
            // Theme + flow are global presentation prefs — shared
            // across all books. lastLocation stays per-work in `saved`.
            const prefs = loadGlobalPrefs();
            const prefFlow = prefs.flow ?? saved.flow;
            const prefTheme = prefs.theme ?? saved.theme;
            window.__JP_READER._flow = prefFlow === "scrolled" ? "scrolled" : "paginated";
            window.__JP_READER._theme =
                prefTheme === "dark" || prefTheme === "sepia" ? prefTheme : "light";

            const view = document.createElement("foliate-view");
            container.append(view);

            // Wheel events fire inside the section iframe and don't
            // bubble out. Bind on each iframe document as it loads, on
            // the outer stage container for chrome scrolls, and on the
            // renderer's #container as a backstop. The shared listener
            // dedupes via __jpWheelAttached.
            view.addEventListener("load", (e) => {
                const doc = e.detail?.doc;
                const index = e.detail?.index;
                if (doc) {
                    captureLayoutFromDoc(doc);
                    attachWheel(doc);
                    attachKeyNav(doc);
                    attachLookupHover(doc);
                    if (doc.defaultView) {
                        attachWheel(doc.defaultView);
                        attachKeyNav(doc.defaultView);
                    }
                    if (typeof index === "number") {
                        // Always replace — the live-rendered doc is the
                        // most authoritative source for this section's
                        // chapter markers (font shaping etc. is irrelevant
                        // since we read textContent).
                        setSectionChapters(index, extractChaptersFromDoc(doc));
                    }
                }
                if (typeof onLoad === "function") onLoad(e.detail);
            });

            view.addEventListener("relocate", (e) => {
                const savedLocation = savedLocationFromRelocate(e.detail);
                if (savedLocation != null) {
                    saveReaderState(workId, { lastLocation: savedLocation });
                }
                // Track latest detail so bookmark UI can snapshot it
                // without reaching into renderer internals.
                window.__JP_READER._lastRelocateDetail = e.detail;
                if (typeof onRelocate === "function") onRelocate(e.detail);
            });
            attachWheel(container);
            const bindRenderer = () => {
                const renderer = view.renderer;
                if (!renderer) return false;
                attachWheel(renderer);
                const innerContainer = renderer.shadowRoot?.getElementById("container");
                if (innerContainer) attachWheel(innerContainer);
                return true;
            };
            if (!bindRenderer()) setTimeout(bindRenderer, 100);
            // Catch the first iframe if it was already inserted before
            // we registered the load handler.
            setTimeout(() => attachAllIframes(view), 200);

            const file = blob instanceof File
                ? blob
                : new File([blob], "book.epub", { type: "application/epub+zip" });

            console.log("[reader-init] opening EPUB", file);
            await view.open(file);
            console.log("[reader-init] view.open resolved", view);

            // Pre-walk every section to seed the chapter cache before
            // the user can interact. Foliate's `load` event only fires
            // for the section it paginates into; without this, the
            // cache stays empty for any chapter the user hasn't
            // visited yet. createDocument() returns a parsed Document
            // we can query immediately and then drop.
            try {
                const sections = view.book?.sections ?? [];
                await Promise.all(sections.map(async (section, idx) => {
                    if (!section || section.linear === "no") return;
                    if (typeof section.createDocument !== "function") return;
                    if (window.__JP_READER._chaptersBySection.has(idx)) return;
                    try {
                        const doc = await section.createDocument();
                        if (doc) {
                            setSectionChapters(idx, extractChaptersFromDoc(doc));
                        }
                    } catch (e) {
                        // Skip unreadable sections rather than aborting
                        // the whole pre-walk.
                    }
                }));
            } catch (e) {
                console.warn("[reader-init] chapter pre-walk failed", e);
            }

            // Default to a comfortable column width for vertical Japanese reading.
            const renderer = view.renderer;
            if (renderer) {
                const initialFlow = window.__JP_READER._flow || "paginated";
                renderer.setAttribute("flow", initialFlow);
                renderer.setAttribute("animated", "");
                renderer.setAttribute("gap", "5%");
                applyFlowSizing(renderer, initialFlow);
                // Force a readable theme; Aozora's EPUB CSS hard-codes
                // black-on-white which becomes invisible against a dark
                // app chrome.
                const initialTheme = window.__JP_READER._theme || "light";
                applyChromeColors(initialTheme);
                window.__JP_READER._lastView = view;
                reapplyStyles();
                await view.init({
                    lastLocation: saved.lastLocation ?? null,
                    showTextStart: false,
                });
            } else {
                console.warn("[reader-init] view.renderer not set after open");
            }

            window.__JP_READER._lastView = view;
            // Live chapter highlight tracker — feeds the registered
            // callback whenever the computed flat chapter index
            // changes. Independent of relocate's 250ms debounce.
            window.__JP_READER._lastChapterIdx = null;
            installChapterTracker(view);
            return {
                view,
                flow: window.__JP_READER._flow || "paginated",
                theme: window.__JP_READER._theme || "light",
            };
        } catch (err) {
            console.error("[reader-init] mount failed", err);
            throw err;
        }
    },

    /** Navigate the most recently-mounted view. */
    next(view) { (view ?? window.__JP_READER._lastView)?.next?.(); },
    prev(view) { (view ?? window.__JP_READER._lastView)?.prev?.(); },
    goTo(view, target) { (view ?? window.__JP_READER._lastView)?.goTo?.(target); },
    handleReaderAction(action, key) { return performReaderAction(action, key); },

    /**
     * Section-only chapter lookup. Used by the BookmarkRow render so
     * pre-existing bookmarks (where `chapter` was stored empty)
     * pick up the new inline-markup labels at display time.
     * Returns a string label or null.
     */
    resolveChapterForSection(sectionIndex) {
        return composeChapterLabel(sectionChapterDefaults(sectionIndex));
    },

    /**
     * Flat list of every harvested chapter heading across the whole
     * book in section order. Each entry is { sectionIndex, level,
     * label, id, indexInSection }. Used by the chapters drawer.
     */
    getChapterList() {
        const map = window.__JP_READER?._chaptersBySection;
        if (!(map instanceof Map)) return [];
        const indices = Array.from(map.keys()).sort((a, b) => a - b);
        const out = [];
        for (const idx of indices) {
            const list = map.get(idx) || [];
            for (const ch of list) {
                if (!ch || !ch.label) continue;
                out.push({
                    sectionIndex: idx,
                    level: ch.level,
                    label: ch.label,
                    id: ch.id || null,
                    indexInSection: typeof ch.indexInSection === "number"
                        ? ch.indexInSection
                        : null,
                });
            }
        }
        return out;
    },

    /**
     * Register a callback invoked whenever the live chapter tracker
     * detects a change (rAF-throttled, scroll-driven — independent
     * of relocate's debounce). Callback receives the new flat index
     * (or null) and runs once at install time to prime the UI.
     */
    setChapterChangeCallback(cb) {
        window.__JP_READER._chapterChangeCb =
            typeof cb === "function" ? cb : null;
        // Immediately fire with the current value so the UI doesn't
        // have to wait for the next scroll/relocate tick.
        if (typeof cb === "function") {
            try { cb(window.__JP_READER._lastChapterIdx ?? null); }
            catch (e) { console.warn("[reader-init] chapter cb prime threw", e); }
        }
    },

    /**
     * Pin the current-chapter flat index to `idx` for `ms` ms (default
     * 500). While the pin is active, getCurrentChapterIndex returns
     * the pinned value, which lets the chapter drawer's row-click
     * handler force the highlight onto the clicked chapter without
     * the relocate-debounced detector immediately overriding it with
     * whatever range.startContainer happens to resolve to after the
     * jump (which can be the next chapter when the goTo anchor
     * lands just past the heading element).
     */
    pinCurrentChapter(idx, ms) {
        if (typeof idx !== "number" || idx < 0) {
            window.__JP_READER._pinnedChapter = null;
            return;
        }
        const timeout = typeof ms === "number" && ms > 0 ? ms : 500;
        window.__JP_READER._pinnedChapter = {
            flatIndex: idx,
            expires: Date.now() + timeout,
        };
    },

    /**
     * Flat-list index (0-based) of the chapter the user is currently
     * inside, or null if unknown. Uses the latest relocate detail so
     * it stays accurate without re-scanning every render.
     */
    getCurrentChapterIndex() {
        const pinned = window.__JP_READER._pinnedChapter;
        if (pinned && Date.now() < pinned.expires) {
            return pinned.flatIndex;
        }
        const detail = window.__JP_READER._lastRelocateDetail;
        if (!detail) return null;
        const sectionIndex = detail?.section?.current;
        if (typeof sectionIndex !== "number") return null;
        const flat = this.getChapterList();
        if (!flat.length) return null;
        const range = detail?.range;
        const doc = range?.startContainer?.ownerDocument ?? null;
        const docNodes = doc ? doc.querySelectorAll(CHAPTER_SELECTOR) : null;

        let candidate = -1;
        for (let i = 0; i < flat.length; i++) {
            const e = flat[i];
            if (e.sectionIndex < sectionIndex) {
                candidate = i;
                continue;
            }
            if (e.sectionIndex > sectionIndex) break;
            // Same section as the user — compare DOM positions if we can.
            if (!doc || !range || !docNodes) {
                // Best effort: pick the first chapter of the section
                // when we have no range to compare against.
                if (candidate < 0) candidate = i;
                continue;
            }
            const el = docNodes[e.indexInSection];
            if (!el) continue;
            let before;
            try {
                const cmp = el.compareDocumentPosition(range.startContainer);
                before = (cmp & Node.DOCUMENT_POSITION_FOLLOWING) !== 0 || cmp === 0;
            } catch {
                continue;
            }
            if (before) candidate = i;
            else break;
        }
        return candidate >= 0 ? candidate : null;
    },

    /**
     * For the bookmarks drawer: "Chapter X of Y" where X is the
     * 1-based position of the latest chapter at or before the given
     * section index, and Y is the total chapter count. Returns null
     * when the cache is empty.
     */
    getChapterPosition(sectionIndex) {
        const list = window.__JP_READER.getChapterList();
        if (!list.length) return null;
        const target = typeof sectionIndex === "number" ? sectionIndex : 0;
        let best = -1;
        for (let i = 0; i < list.length; i++) {
            if (list[i].sectionIndex <= target) best = i;
            else break;
        }
        if (best < 0) return null;
        return { current: best + 1, total: list.length };
    },

    /**
     * Jump the reader to a specific chapter heading. AozoraEpub3 doesn't
     * give chap divs DOM ids, so we navigate by chapter index within
     * the section — querySelectorAll on the rendered iframe doc
     * returns the same order we used at extraction time. Falls back
     * to id when present, then to "top of section".
     */
    goToChapter(sectionIndex, chapterId, indexInSection) {
        const view = window.__JP_READER?._lastView;
        const renderer = view?.renderer;
        if (!renderer || typeof renderer.goTo !== "function") return;
        const idx = typeof sectionIndex === "number" ? sectionIndex : 0;
        let anchor;
        if (typeof indexInSection === "number" && indexInSection >= 0) {
            anchor = (doc) => {
                const list = doc.querySelectorAll(CHAPTER_SELECTOR);
                return list[indexInSection] ?? 0;
            };
        } else if (typeof chapterId === "string" && chapterId.length > 0) {
            anchor = (doc) => doc.getElementById(chapterId) ?? 0;
        } else {
            anchor = () => 0;
        }
        Promise.resolve(renderer.goTo({ index: idx, anchor })).catch((e) => {
            console.warn("[reader-init] goToChapter failed", e);
        });
    },

    /** Cycle theme: light → dark → sepia → light. */
    cycleTheme() {
        const order = ["light", "dark", "sepia"];
        const cur = window.__JP_READER._theme || "light";
        const next = order[(order.indexOf(cur) + 1) % order.length];
        window.__JP_READER._theme = next;
        saveGlobalPrefs({ theme: next });
        applyChromeColors(next);
        reapplyStyles();
        return next;
    },

    setTheme(theme) {
        window.__JP_READER._theme = theme;
        saveGlobalPrefs({ theme });
        applyChromeColors(theme);
        reapplyStyles();
    },

    setFontScale(scale) {
        window.__JP_READER._fontScale = scale;
        reapplyStyles();
    },

    setLineHeight(lh) {
        window.__JP_READER._lineHeight = lh;
        reapplyStyles();
    },

    /**
     * In scrolled mode for vertical-writing books, the document scrolls
     * horizontally. Translate vertical wheel deltas onto the renderer's
     * inner scroll container so a normal mouse-wheel still advances the
     * reading position.
     */
    wheelScroll(deltaX, deltaY) {
        feedScrolledWheel(wheelDeltaForScrolledMode(deltaX, deltaY));
    },

    /**
     * Snapshot of the current reading location for the bookmark layer.
     * Returns `null` if nothing has relocated yet.
     */
    getCurrentLocation() {
        const detail = window.__JP_READER._lastRelocateDetail;
        if (!detail) return null;
        const view = window.__JP_READER._lastView;
        const flat = this.getChapterList();
        const chapterIdx = this.getCurrentChapterIndex();
        return {
            cfi: typeof detail.cfi === "string" ? detail.cfi : null,
            // Foliate's view-level relocate spreads SectionProgress
            // (which has .section.current), not the raw paginator
            // `index`. Read from there.
            sectionIndex: typeof detail?.section?.current === "number"
                ? detail.section.current
                : null,
            fraction: typeof detail.fraction === "number" ? detail.fraction : null,
            chapter: inferChapterLabel(view, detail),
            chapterIndex: typeof chapterIdx === "number" ? chapterIdx : null,
            chapterTotal: flat.length || null,
        };
    },

    /** Navigate the reader to a stored bookmark target. */
    goToBookmark(target) {
        const view = window.__JP_READER._lastView;
        if (!view) return;
        if (typeof target === "string" && target.length > 0) {
            view.goTo?.(target);
            return;
        }
        if (target && typeof target.fraction === "number") {
            view.goToFraction?.(target.fraction);
        }
    },

    /** Toggle paginated / scrolled flow. Returns the new flow. */
    toggleFlow() {
        const cur = window.__JP_READER._flow || "paginated";
        const next = cur === "paginated" ? "scrolled" : "paginated";
        window.__JP_READER._flow = next;
        saveGlobalPrefs({ flow: next });
        const renderer = window.__JP_READER._lastView?.renderer;
        if (renderer) {
            renderer.setAttribute("flow", next);
            applyFlowSizing(renderer, next);
        }
        return next;
    },
};

// Quiet a Trunk lint about unused exports:
export const _foliate_loaded = true;
