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
/* In scrolled mode we want the wheel to feel smooth, not staircased.
   Each wheel event fires onWheelInner roughly every 16ms during a
   gesture; we accumulate the deltas and run an rAF loop that eases
   the renderer's #container.scrollLeft toward the target. Easing
   keeps the motion continuous instead of stop/start, and the
   accumulator drains naturally when the user stops scrolling. */
let scrolledTarget = 0;
let scrolledCurrent = 0;
let scrolledAnimating = false;
let scrolledContainer = null;
const SCROLLED_EASING = 0.18;
const SCROLLED_GAIN = 1.0;

const scheduleScrolledScroll = (delta) => {
    const renderer = window.__JP_READER._lastView?.renderer;
    const container = renderer?.shadowRoot?.getElementById("container");
    if (!container) return;
    if (container !== scrolledContainer) {
        scrolledContainer = container;
        scrolledCurrent = container.scrollLeft;
        scrolledTarget = scrolledCurrent;
    }
    scrolledTarget += delta * SCROLLED_GAIN;
    // Clamp into the renderer's actual scroll range so we don't drift
    // off the edges into invisible work.
    const max = container.scrollWidth - container.clientWidth;
    if (scrolledTarget < 0) scrolledTarget = 0;
    if (scrolledTarget > max) scrolledTarget = max;
    if (!scrolledAnimating) {
        scrolledAnimating = true;
        requestAnimationFrame(stepScrolledScroll);
    }
};

const stepScrolledScroll = () => {
    if (!scrolledContainer) {
        scrolledAnimating = false;
        return;
    }
    const diff = scrolledTarget - scrolledCurrent;
    if (Math.abs(diff) < 0.5) {
        // Snap to target, finish.
        scrolledContainer.scrollLeft = scrolledTarget;
        scrolledCurrent = scrolledTarget;
        scrolledAnimating = false;
        return;
    }
    scrolledCurrent += diff * SCROLLED_EASING;
    scrolledContainer.scrollLeft = scrolledCurrent;
    requestAnimationFrame(stepScrolledScroll);
};

/* In paginated mode the user is *not* scrolling, they're flipping
   pages. macOS trackpad inertia keeps sending wheel events for ~1s
   after a single swipe, so a simple time debounce fires the next
   page two or three times. Use a "silence lock": when we navigate,
   set wheelLocked = true and arm a release timer. Every additional
   wheel event re-arms the timer, so inertia *extends* the lock
   instead of breaking through it. Lock releases after WHEEL_QUIET_MS
   of true silence — the user genuinely stopping. */
let wheelLocked = false;
let wheelUnlockTimer = null;
const WHEEL_QUIET_MS = 180;
const WHEEL_MIN_DELTA = 1;

const armWheelLockRelease = () => {
    if (wheelUnlockTimer) clearTimeout(wheelUnlockTimer);
    wheelUnlockTimer = setTimeout(() => {
        wheelLocked = false;
        wheelUnlockTimer = null;
    }, WHEEL_QUIET_MS);
};

const onWheelInner = (ev) => {
    const flow = window.__JP_READER._flow || "paginated";

    if (flow === "scrolled") {
        ev.preventDefault();
        scheduleScrolledScroll(ev.deltaX + ev.deltaY);
        return;
    }

    // Paginated: lock-on-first-event, re-arm on each subsequent.
    ev.preventDefault();
    if (wheelLocked) {
        armWheelLockRelease();
        return;
    }
    const dy = ev.deltaY + ev.deltaX;
    if (Math.abs(dy) < WHEEL_MIN_DELTA) return;
    wheelLocked = true;
    armWheelLockRelease();

    const view = window.__JP_READER._lastView;
    if (!view) return;
    if (dy > 0) view.next?.();
    else view.prev?.();
};

// Some iframes are created before the load event we hook into; sweep
// for them once after mount so we don't miss the very first section.
const attachAllIframes = (root) => {
    if (!root) return;
    const visit = (node) => {
        if (!node) return;
        if (node.tagName === "IFRAME") {
            try {
                if (node.contentDocument) attachWheel(node.contentDocument);
                if (node.contentWindow) attachWheel(node.contentWindow);
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
                    if (doc.defaultView) attachWheel(doc.defaultView);
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
        const renderer = window.__JP_READER._lastView?.renderer;
        const container = renderer?.shadowRoot?.getElementById("container");
        if (!container) return;
        container.scrollBy({ left: deltaX + deltaY, top: 0, behavior: "auto" });
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
