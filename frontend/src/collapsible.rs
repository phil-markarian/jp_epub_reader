//! Persist `<details>` open/closed state across sessions.
//!
//! Each collapsible panel calls `persist_collapse(node_ref, "key")`
//! inside its component setup. On first paint we restore the saved
//! state from localStorage; subsequent toggle events get written
//! back. The store namespace is shared across the app under the
//! `jp-collapsed:` prefix so keys are short ("source", "library",
//! "dictionaries", "index") without colliding with other localStorage
//! keys we use elsewhere.

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

const KEY_PREFIX: &str = "jp-collapsed:";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// Wire a `<details>` element identified by `node_ref` to a
/// localStorage entry keyed by `slug`. Restores the saved state on
/// first attach and writes back on every toggle.
pub fn persist_collapse(node_ref: NodeRef<leptos::html::Details>, slug: &'static str) {
    Effect::new(move |_| {
        let Some(el) = node_ref.get() else { return };
        // NodeRef<html::Details> returns Leptos's HtmlElement<Details>
        // wrapper; pull the underlying web_sys::HtmlElement out and
        // narrow to HtmlDetailsElement via dyn_into.
        let generic: web_sys::HtmlElement = (*el).clone().into();
        let Ok(html_el) = generic.dyn_into::<web_sys::HtmlDetailsElement>() else {
            return;
        };
        let key = format!("{KEY_PREFIX}{slug}");

        // Restore.
        if let Some(s) = storage() {
            if let Ok(Some(saved)) = s.get_item(&key) {
                html_el.set_open(saved == "open");
            }
        }

        // Persist on every toggle.
        let target = html_el.clone();
        let key_for_handler = key.clone();
        let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
            if let Some(s) = storage() {
                let _ = s.set_item(
                    &key_for_handler,
                    if target.open() { "open" } else { "closed" },
                );
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = html_el
            .add_event_listener_with_callback("toggle", cb.as_ref().unchecked_ref());
        // Leak — the listener should live for the whole panel
        // lifetime. The Effect re-runs only when node_ref changes,
        // which doesn't happen after mount.
        cb.forget();
    });
}
