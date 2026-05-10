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
                renderer.setAttribute("flow", "paginated");
                renderer.setAttribute("animated", "");
                renderer.setAttribute("max-inline-size", "720");
                renderer.setAttribute("max-block-size", "1100");
                renderer.setAttribute("gap", "5%");
                // Force a readable theme; Aozora's EPUB CSS hard-codes
                // black-on-white which becomes invisible against a dark
                // app chrome.
                const initialTheme = window.__JP_READER._theme || "light";
                renderer.setStyles?.(themeCSS(initialTheme));
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
        return next;
    },

    setTheme(theme) {
        window.__JP_READER._theme = theme;
        const view = window.__JP_READER._lastView;
        view?.renderer?.setStyles?.(themeCSS(theme));
    },
};

// Quiet a Trunk lint about unused exports:
export const _foliate_loaded = true;
