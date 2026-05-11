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
/* Scrolled mode gets its own transport entirely separate from paginated
   mode. For vertical-writing books, Foliate's own next/prev(distance)
   path already knows how to do smooth in-section movement plus section
   transitions at the edges, so prefer that over manual DOM scrolling. */
let scrolledContainer = null;
const SCROLLED_KEY_STEP = 220;
const SCROLLED_WHEEL_GAIN = 2.2;
const SCROLLED_WHEEL_MAX_RATIO = 0.6;
const SCROLLED_WHEEL_SMOOTHING = 0.2;
const SCROLLED_WHEEL_DECAY = 0.9;
const SCROLLED_WHEEL_MIN_VELOCITY = 0.08;
const SCROLLED_WHEEL_MAX_VELOCITY = 180;
let scrolledWheelVelocity = 0;
let scrolledWheelAnimating = false;
let scrolledWheelCanContinue = true;

const resetScrolledWheelState = () => {
    scrolledWheelVelocity = 0;
    scrolledWheelAnimating = false;
    scrolledWheelCanContinue = true;
};

const isScrolledFlow = () => (window.__JP_READER._flow || "paginated") === "scrolled";
const isVerticalWriting = () => window.__JP_READER._verticalWriting !== false;

const captureLayoutFromDoc = (doc) => {
    if (!doc?.defaultView) return;
    const { writingMode } = doc.defaultView.getComputedStyle(doc.body);
    window.__JP_READER._verticalWriting =
        writingMode === "vertical-rl" || writingMode === "vertical-lr";
};

const wheelDeltaForScrolledMode = (deltaX, deltaY) => {
    const clampWheelDelta = (delta) => {
        const max = getScrolledKeyStep() * SCROLLED_WHEEL_MAX_RATIO;
        return Math.sign(delta) * Math.min(Math.abs(delta), max);
    };

    // Current primary target is vertical-writing books in scrolled mode,
    // which move horizontally. Map gestures to visible left/right intent:
    // down/left => left, up/right => right.
    if (isVerticalWriting()) {
        return clampWheelDelta((deltaY - deltaX) * SCROLLED_WHEEL_GAIN);
    }
    // Fallback for horizontal-writing content: keep a simple primary-axis
    // mapping without affecting the vertical-writing path.
    const primary = Math.abs(deltaY) >= Math.abs(deltaX) ? deltaY : deltaX;
    return clampWheelDelta(primary * SCROLLED_WHEEL_GAIN);
};

const getScrolledContainer = () => {
    const renderer = window.__JP_READER._lastView?.renderer;
    const container = renderer?.shadowRoot?.getElementById("container");
    return { renderer, container };
};

const applyScrolledWheelStep = (delta) => {
    const { container } = getScrolledContainer();
    if (!container) return false;
    if (container !== scrolledContainer) {
        scrolledContainer = container;
    }

    // Drive the inner overflow:auto #container directly. Foliate's
    // Paginator.scrollBy assumes #scrollBounds is populated by the
    // paginated codepath; in flow="scrolled" it can be undefined and
    // the call no-ops or throws. Native scrollBy on the DOM element
    // is the deterministic target.
    const beforeLeft = container.scrollLeft;
    const beforeTop = container.scrollTop;
    if (isVerticalWriting()) {
        container.scrollBy({ left: delta, top: 0, behavior: "auto" });
    } else {
        container.scrollBy({ left: 0, top: delta, behavior: "auto" });
    }
    const afterLeft = container.scrollLeft;
    const afterTop = container.scrollTop;
    return (
        Math.abs(afterLeft - beforeLeft) > 0.5 ||
        Math.abs(afterTop - beforeTop) > 0.5
    );
};

const scrollScrolledByKey = (delta) => {
    const { container } = getScrolledContainer();
    if (!container) return false;
    if (container !== scrolledContainer) {
        scrolledContainer = container;
    }
    // Keyboard movement is independent from the wheel momentum loop.
    resetScrolledWheelState();
    const opts = isVerticalWriting()
        ? { left: delta, top: 0, behavior: "smooth" }
        : { left: 0, top: delta, behavior: "smooth" };
    container.scrollBy(opts);
    return true;
};

const scrollScrolledBy = (delta, smooth = false) => {
    const { container } = getScrolledContainer();
    if (!container) return false;
    if (container !== scrolledContainer) {
        scrolledContainer = container;
    }
    const behavior = smooth ? "smooth" : "auto";
    const opts = isVerticalWriting()
        ? { left: delta, top: 0, behavior }
        : { left: 0, top: delta, behavior };
    container.scrollBy(opts);
    return true;
};

const getScrolledKeyStep = () => {
    const { container } = getScrolledContainer();
    if (!container) return 160;
    return isVerticalWriting()
        ? Math.max(SCROLLED_KEY_STEP, container.clientWidth * 0.35)
        : Math.max(SCROLLED_KEY_STEP, container.clientHeight * 0.35);
};

const smoothScrollLeft = () => scrollScrolledByKey(getScrolledKeyStep());
const smoothScrollRight = () => scrollScrolledByKey(-getScrolledKeyStep());

const scheduleScrolledWheel = (delta) => {
    if (Math.abs(delta) < 0.01) return;
    scrolledWheelVelocity += delta;
    scrolledWheelVelocity = Math.sign(scrolledWheelVelocity) *
        Math.min(Math.abs(scrolledWheelVelocity), SCROLLED_WHEEL_MAX_VELOCITY);
    if (scrolledWheelAnimating) return;
    scrolledWheelAnimating = true;

    const tick = () => {
        const step = scrolledWheelVelocity * SCROLLED_WHEEL_SMOOTHING;
        if (Math.abs(step) >= 0.01) {
            scrolledWheelCanContinue = applyScrolledWheelStep(step);
        }
        scrolledWheelVelocity *= SCROLLED_WHEEL_DECAY;
        if (Math.abs(scrolledWheelVelocity) < SCROLLED_WHEEL_MIN_VELOCITY) {
            resetScrolledWheelState();
            return;
        }
        requestAnimationFrame(tick);
    };

    requestAnimationFrame(tick);
};

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
        const delta = wheelDeltaForScrolledMode(ev.deltaX, ev.deltaY);
        if (!scrolledWheelCanContinue && Math.abs(delta) >= 0.5) {
            resetScrolledWheelState();
            scrollScrolledBy(delta, false);
            return;
        }
        scheduleScrolledWheel(delta);
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
        // In horizontal/scrolled mode, use literal left/right movement
        // with smooth scrolling rather than logical prev/next pages.
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
                if (doc) {
                    captureLayoutFromDoc(doc);
                    attachWheel(doc);
                    attachKeyNav(doc);
                    if (doc.defaultView) {
                        attachWheel(doc.defaultView);
                        attachKeyNav(doc.defaultView);
                    }
                }
                if (typeof onLoad === "function") onLoad(e.detail);
            });

            view.addEventListener("relocate", (e) => {
                const savedLocation = savedLocationFromRelocate(e.detail);
                if (savedLocation != null) {
                    saveReaderState(workId, { lastLocation: savedLocation });
                }
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
        scheduleScrolledWheel(wheelDeltaForScrolledMode(deltaX, deltaY));
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
