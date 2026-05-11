// Module-loaded helper that wires up foliate-js so the wasm reader can
// drop an EPUB blob in and get a rendered <foliate-view>. Loaded as a
// type=module script from index.html. Importing view.js registers the
// <foliate-view> custom element as a side effect.

import "./foliate-js/view.js";

const VERTICAL_DIR = "rtl"; // tategaki books page right-to-left

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
/* Smooth-scroll with inertia for scrolled mode. macOS trackpad
   inertia events don't reach the webview through Tauri, so each
   wheel event adds to a velocity instead of moving the page
   directly; a rAF loop applies velocity to scrollLeft each frame
   and decays it geometrically. The result feels like native momentum
   scrolling regardless of whether the user is on a trackpad or a
   click-wheel mouse. */
let scrolledVelocity = 0;
let scrolledContainer = null;
let scrolledRunning = false;
// Tuning: at 60fps, decay 0.94 ≈ 0.025x after one second (≈40 frames
// to bleed off most of the velocity). Gain 0.55 keeps wheel feel near
// native — a typical click-wheel tick (~120px) lands you a comfortable
// fraction of a column over the next ~0.3s.
const SCROLLED_DECAY = 0.94;
const SCROLLED_GAIN = 0.55;
const SCROLLED_VELOCITY_FLOOR = 0.25;
const SCROLLED_KEY_STEP = 120;

// In horizontal scrolled reading, treat both vertical and horizontal
// trackpad gestures as left/right intent:
// - down  => move left
// - up    => move right
// - left  => move left
// - right => move right
const horizontalGestureDelta = (deltaX, deltaY) => deltaX - deltaY;

const applyScrolledDelta = (delta) => {
    const renderer = window.__JP_READER._lastView?.renderer;
    const container = renderer?.shadowRoot?.getElementById("container");
    if (!renderer || !container) return false;
    if (container !== scrolledContainer) {
        scrolledContainer = container;
    }
    // Feed the same delta into both axes and let Foliate pick the
    // correct scroll axis internally based on the book's writing mode.
    renderer.scrollBy?.(delta, delta);
    return true;
};

const scheduleScrolledScroll = (delta) => {
    if (!applyScrolledDelta(0)) return;
    // Add delta to current velocity instead of overwriting so fast
    // repeated flicks accelerate.
    scrolledVelocity += delta * SCROLLED_GAIN;
    if (!scrolledRunning) {
        scrolledRunning = true;
        requestAnimationFrame(stepScrolledScroll);
    }
};

const stepScrolledScroll = () => {
    if (!scrolledContainer) {
        scrolledRunning = false;
        return;
    }
    applyScrolledDelta(scrolledVelocity);
    scrolledVelocity *= SCROLLED_DECAY;
    if (Math.abs(scrolledVelocity) < SCROLLED_VELOCITY_FLOOR) {
        scrolledVelocity = 0;
        scrolledRunning = false;
        return;
    }
    requestAnimationFrame(stepScrolledScroll);
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
    const flow = window.__JP_READER._flow || "paginated";

    if (flow === "scrolled") {
        ev.preventDefault();
        scheduleScrolledScroll(horizontalGestureDelta(ev.deltaX, ev.deltaY));
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

    const flow = window.__JP_READER._flow || "paginated";
    if (flow === "scrolled") {
        // In horizontal/scrolled mode, left/right should be literal
        // movement directions rather than logical prev/next page.
        if (key === "ArrowLeft") {
            applyScrolledDelta(-SCROLLED_KEY_STEP);
            return true;
        }
        if (key === "ArrowRight") {
            applyScrolledDelta(SCROLLED_KEY_STEP);
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
    async mount(container, blob, onRelocate, onLoad) {
        try {
            // Reset container so re-entries don't stack views.
            container.replaceChildren();

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
                    attachWheel(doc);
                    attachKeyNav(doc);
                    if (doc.defaultView) {
                        attachWheel(doc.defaultView);
                        attachKeyNav(doc.defaultView);
                    }
                }
                if (typeof onLoad === "function") onLoad(e.detail);
            });

            if (typeof onRelocate === "function") {
                view.addEventListener("relocate", (e) => onRelocate(e.detail));
            }
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
                // view.open() registers the book but doesn't paint
                // anything; renderer.next() navigates to the first
                // section.
                renderer.next?.();
            } else {
                console.warn("[reader-init] view.renderer not set after open");
            }

            window.__JP_READER._lastView = view;
            return { view };
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
        applyChromeColors(next);
        reapplyStyles();
        return next;
    },

    setTheme(theme) {
        window.__JP_READER._theme = theme;
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
        applyScrolledDelta(horizontalGestureDelta(deltaX, deltaY));
    },

    /** Toggle paginated / scrolled flow. Returns the new flow. */
    toggleFlow() {
        const cur = window.__JP_READER._flow || "paginated";
        const next = cur === "paginated" ? "scrolled" : "paginated";
        window.__JP_READER._flow = next;
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
