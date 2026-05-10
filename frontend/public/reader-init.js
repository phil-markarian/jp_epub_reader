// Module-loaded helper that wires up foliate-js so the wasm reader can
// drop an EPUB blob in and get a rendered <foliate-view>. Loaded as a
// type=module script from index.html. Importing view.js registers the
// <foliate-view> custom element as a side effect.

import "./foliate-js/view.js";

const VERTICAL_DIR = "rtl"; // tategaki books page right-to-left

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
            } else {
                console.warn("[reader-init] view.renderer not set after open");
            }

            return { view };
        } catch (err) {
            console.error("[reader-init] mount failed", err);
            throw err;
        }
    },

    /** Navigate the most recently-mounted view. */
    next(view) { view?.next?.(); },
    prev(view) { view?.prev?.(); },
    goTo(view, target) { view?.goTo?.(target); },
};

// Quiet a Trunk lint about unused exports:
export const _foliate_loaded = true;
