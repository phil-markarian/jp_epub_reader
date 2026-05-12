// Module-loaded helper that wires up foliate-js so the wasm reader can
// drop an EPUB blob in and get a rendered <foliate-view>. Loaded as a
// type=module script from index.html. Importing view.js registers the
// <foliate-view> custom element as a side effect.

import "./foliate-js/view.js";

const VERTICAL_DIR = "rtl"; // tategaki books page right-to-left
const READER_STATE_VERSION = 1;

const readerStateKey = (workId) => `jp-reader-state:v${READER_STATE_VERSION}:${workId}`;

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
 */
const findChaptersAtOrBefore = (list, doc, range) => {
    if (!Array.isArray(list) || list.length === 0) return null;
    if (!range || !doc || !range.startContainer) return null;
    if (range.startContainer.ownerDocument !== doc
        && range.startContainer.getRootNode?.() !== doc) {
        return null;
    }
    let chap1 = null;
    let chap2 = null;
    let chap3 = null;
    let lastLevel = 0;
    for (const ch of list) {
        const el = ch.id ? doc.getElementById(ch.id) : null;
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
    // If no chap1 was seen but a lower-level heading exists (e.g.
    // 夢十夜 is all chap2), use those.
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
 *   1. `detail.tocItem.label` if present (Foliate's own resolver).
 *   2. Inline-markup cache for the current section, indexed against
 *      `detail.range`. If the range is missing or in a different
 *      document, fall back to section-defaults.
 *   3. `book.toc` href-resolution walk by section index.
 */
const inferChapterLabel = (view, detail) => {
    const fromDetail = detail?.tocItem?.label;
    if (typeof fromDetail === "string" && fromDetail.trim().length > 0) {
        return fromDetail.trim();
    }
    const currentIndex = detail?.section?.current;
    const map = window.__JP_READER?._chaptersBySection;
    if (map instanceof Map && typeof currentIndex === "number") {
        const list = map.get(currentIndex) || [];
        const range = detail?.range;
        const doc = range?.startContainer?.ownerDocument ?? null;
        let parts = list.length ? findChaptersAtOrBefore(list, doc, range) : null;
        // If we couldn't compare against the range (no range, or
        // section content not currently in this doc), or the current
        // section had no chap1 in the at-or-before window, fall back
        // to defaults that walk earlier sections for chap1.
        if (!parts || parts.chap1 == null) {
            const defaults = sectionChapterDefaults(currentIndex);
            if (defaults) {
                // Merge: keep finer chap2/chap3 from range walk if
                // present, but inherit chap1 from defaults.
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
        const composed = composeChapterLabel(parts);
        if (composed) return composed;
    }
    return tocWalkLabel(view, currentIndex);
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
            window.__JP_READER._flow = saved.flow === "scrolled" ? "scrolled" : "paginated";
            window.__JP_READER._theme =
                saved.theme === "dark" || saved.theme === "sepia" ? saved.theme : "light";

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
        saveReaderState(window.__JP_READER._workId, { theme: next });
        applyChromeColors(next);
        reapplyStyles();
        return next;
    },

    setTheme(theme) {
        window.__JP_READER._theme = theme;
        saveReaderState(window.__JP_READER._workId, { theme });
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
        saveReaderState(window.__JP_READER._workId, { flow: next });
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
