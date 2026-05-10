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
        // block = horizontal (column width).
        renderer.setAttribute("max-inline-size", "900");
        renderer.setAttribute("max-block-size", "1400");
        renderer.setAttribute("max-column-count", "2");
    }
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

            if (typeof onRelocate === "function") {
                view.addEventListener("relocate", (e) => onRelocate(e.detail));
            }
            if (typeof onLoad === "function") {
                view.addEventListener("load", (e) => onLoad(e.detail));
            }

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
                renderer.setStyles?.(themeCSS(initialTheme));
                applyChromeColors(initialTheme);
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
        const view = window.__JP_READER._lastView;
        view?.renderer?.setStyles?.(themeCSS(next));
        applyChromeColors(next);
        return next;
    },

    setTheme(theme) {
        window.__JP_READER._theme = theme;
        const view = window.__JP_READER._lastView;
        view?.renderer?.setStyles?.(themeCSS(theme));
        applyChromeColors(theme);
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
