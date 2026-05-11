//! Phase 3 reader window. Renders an EPUB through foliate-js inside a
//! per-work webview window. Phase 5 will add dictionary lookup over
//! click events captured here.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    /// Defined by `frontend/public/reader-init.js`. Mounts a foliate
    /// `<foliate-view>` into the container and opens the EPUB.
    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "mount", catch)]
    async fn jp_mount(
        container: web_sys::Element,
        blob: web_sys::Blob,
        work_id: u32,
        on_relocate: JsValue,
        on_load: JsValue,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "cycleTheme")]
    fn jp_cycle_theme() -> JsValue;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "toggleFlow")]
    fn jp_toggle_flow() -> JsValue;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "next")]
    fn jp_next();

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "prev")]
    fn jp_prev();

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "handleReaderAction")]
    fn jp_handle_reader_action(action: &str, key: &str) -> bool;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "setFontScale")]
    fn jp_set_font_scale(scale: f64);

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "setLineHeight")]
    fn jp_set_line_height(lh: f64);
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LibraryEntry {
    work_id: u32,
    #[serde(default)]
    source_id: String,
    title: String,
    author: Option<String>,
    epub_path: String,
    #[serde(default)]
    raw_text_path: Option<String>,
    added_at: i64,
    #[serde(default)]
    last_opened_at: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkArgs {
    work_id: u32,
}

#[derive(Clone, Debug, Deserialize)]
struct MountResult {
    flow: Option<String>,
    theme: Option<String>,
}

const KEYBINDS_LS_KEY: &str = "jp-reader-keybinds";
const KEYBINDS_VERSION_KEY: &str = "jp-reader-keybinds-version";
/// Bump when changing default keybindings to auto-reset any stale
/// localStorage entries from the earlier development sessions.
const KEYBINDS_VERSION: &str = "2";

/// Action ids and their default keys. Each action shows up as one row
/// in the settings popover.
#[allow(clippy::type_complexity)]
const ACTION_DEFAULTS: &[(&str, &str, &[&str])] = &[
    ("next", "Next page", &["ArrowRight", "ArrowDown", "j", "l", " "]),
    ("prev", "Previous page", &["ArrowLeft", "ArrowUp", "k", "h"]),
    ("toggle_flow", "Toggle flow", &["t"]),
    ("cycle_theme", "Cycle theme", &["d"]),
    ("font_up", "Larger text", &["+", "="]),
    ("font_down", "Smaller text", &["-", "_"]),
    ("toggle_settings", "Settings", &[","]),
];

type Keybinds = BTreeMap<String, Vec<String>>;

fn defaults() -> Keybinds {
    ACTION_DEFAULTS
        .iter()
        .map(|(id, _, keys)| (id.to_string(), keys.iter().map(|k| (*k).to_string()).collect()))
        .collect()
}

fn load_keybinds() -> Keybinds {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
        return defaults();
    };
    let version = storage.get_item(KEYBINDS_VERSION_KEY).ok().flatten();
    if version.as_deref() != Some(KEYBINDS_VERSION) {
        // Stale localStorage from an older default scheme — wipe and
        // start over so the user gets the latest sane bindings.
        let _ = storage.remove_item(KEYBINDS_LS_KEY);
        let _ = storage.set_item(KEYBINDS_VERSION_KEY, KEYBINDS_VERSION);
        return defaults();
    }
    if let Some(json) = storage.get_item(KEYBINDS_LS_KEY).ok().flatten() {
        if let Ok(map) = serde_json::from_str::<Keybinds>(&json) {
            // Make sure new actions added in code show up with their defaults.
            let mut merged = defaults();
            for (k, v) in map {
                merged.insert(k, v);
            }
            return merged;
        }
    }
    defaults()
}

fn save_keybinds(kb: &Keybinds) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        if let Ok(json) = serde_json::to_string(kb) {
            let _ = storage.set_item(KEYBINDS_LS_KEY, &json);
        }
    }
}

fn match_action(kb: &Keybinds, key: &str) -> Option<String> {
    for (action, keys) in kb {
        if keys.iter().any(|k| k == key) {
            return Some(action.clone());
        }
    }
    None
}

fn label_key(k: &str) -> String {
    match k {
        " " => "Space".into(),
        "ArrowRight" => "→".into(),
        "ArrowLeft" => "←".into(),
        "ArrowUp" => "↑".into(),
        "ArrowDown" => "↓".into(),
        "Escape" => "Esc".into(),
        s if s.len() == 1 => s.to_uppercase(),
        s => s.into(),
    }
}

#[component]
pub fn ReaderApp(work_id: u32) -> impl IntoView {
    let (entry, set_entry) = signal::<Option<LibraryEntry>>(None);
    let (status, set_status) = signal::<String>("Loading…".into());
    let (theme, set_theme) = signal::<&'static str>("light");
    let (flow, set_flow) = signal::<&'static str>("paginated");
    let stage_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    spawn_local(async move {
        let v = serde_wasm_bindgen::to_value(&WorkArgs { work_id }).unwrap();
        match invoke("get_library_entry", v).await {
            Ok(v) => match serde_wasm_bindgen::from_value::<LibraryEntry>(v) {
                Ok(e) => set_entry.set(Some(e)),
                Err(e) => set_status.set(format!("decode entry: {e}")),
            },
            Err(v) => set_status.set(stringify(&v)),
        }
    });

    // Once both the LibraryEntry and the stage div are mounted, fetch
    // the EPUB bytes and hand off to foliate-js.
    Effect::new(move |_| {
        let Some(_e) = entry.get() else { return };
        let Some(stage) = stage_ref.get() else { return };
        let stage_el: web_sys::Element = (*stage).clone().into();

        spawn_local(async move {
            set_status.set("Fetching book…".into());
            let args = serde_wasm_bindgen::to_value(&WorkArgs { work_id }).unwrap();
            let bytes = match invoke("read_epub_bytes", args).await {
                Ok(v) => v,
                Err(v) => {
                    set_status.set(format!("read_epub_bytes: {}", stringify(&v)));
                    return;
                }
            };
            let blob = match make_blob(&bytes) {
                Ok(b) => b,
                Err(e) => {
                    set_status.set(format!("blob: {e}"));
                    return;
                }
            };

            set_status.set("Rendering…".into());
            match jp_mount(stage_el, blob, work_id, JsValue::NULL, JsValue::NULL).await {
                Ok(v) => {
                    if let Ok(result) = serde_wasm_bindgen::from_value::<MountResult>(v) {
                        if let Some(flow_name) = result.flow {
                            let restored: &'static str =
                                if flow_name == "scrolled" { "scrolled" } else { "paginated" };
                            set_flow.set(restored);
                        }
                        if let Some(theme_name) = result.theme {
                            let restored: &'static str = match theme_name.as_str() {
                                "dark" => "dark",
                                "sepia" => "sepia",
                                _ => "light",
                            };
                            set_theme.set(restored);
                        }
                    }
                    set_status.set(String::new());
                }
                Err(v) => set_status.set(format!("mount: {}", stringify(&v))),
            }
        });
    });

    let on_cycle_theme = move |_| {
        let v = jp_cycle_theme();
        let next = v.as_string().unwrap_or_else(|| "light".into());
        let next: &'static str = match next.as_str() {
            "dark" => "dark",
            "sepia" => "sepia",
            _ => "light",
        };
        set_theme.set(next);
    };
    let on_toggle_flow = move |_| {
        let v = jp_toggle_flow();
        let next = v.as_string().unwrap_or_else(|| "paginated".into());
        let next: &'static str = if next == "scrolled" { "scrolled" } else { "paginated" };
        set_flow.set(next);
    };
    let on_prev = move |_| jp_prev();
    let on_next = move |_| jp_next();

    // Wheel handling lives in reader-init.js (it has to attach inside
    // each section iframe; outer listeners don't see those events).
    let (settings_open, set_settings_open) = signal::<bool>(false);
    let (font_scale, set_font_scale) = signal::<f64>(1.0);
    let (line_height, set_line_height) = signal::<f64>(1.7);

    let on_toggle_settings = move |_| set_settings_open.update(|v| *v = !*v);

    // Keybind state (loaded from localStorage; falls back to defaults).
    let (keybinds, set_keybinds) = signal::<Keybinds>(load_keybinds());
    // When `Some(action)`, the next keypress is recorded for that action.
    let (capturing, set_capturing) = signal::<Option<String>>(None);

    // Keyboard handler. Attached once via Effect; reads the latest signals.
    let last_key_ms = StoredValue::new(0.0_f64);
    Effect::new(move |_| {
        let Some(window) = web_sys::window() else { return };
        let cb = Closure::wrap(Box::new(move |ev: web_sys::KeyboardEvent| {
            if ev.meta_key() || ev.ctrl_key() || ev.alt_key() {
                return;
            }
            let key = ev.key();

            // Let the JS reader layer own raw arrow-key behavior first so
            // scrolled vs paginated mode can diverge cleanly without the
            // Rust keybind map forcing everything through next/prev.
            if matches!(
                key.as_str(),
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown"
            ) && jp_handle_reader_action("arrow", &key)
            {
                ev.prevent_default();
                return;
            }

            // Capture mode: record this key for the pending action.
            if let Some(action) = capturing.get_untracked() {
                ev.prevent_default();
                if key == "Escape" {
                    set_capturing.set(None);
                    return;
                }
                set_keybinds.update(|kb| {
                    let entry = kb.entry(action.clone()).or_default();
                    if !entry.iter().any(|k| k == &key) {
                        entry.push(key.clone());
                    }
                });
                save_keybinds(&keybinds.get_untracked());
                set_capturing.set(None);
                return;
            }

            // Auto-repeat throttle.
            let now = js_sys::Date::now();
            if ev.repeat() && now - last_key_ms.get_value() < 80.0 {
                return;
            }
            last_key_ms.set_value(now);

            let kb = keybinds.get_untracked();
            let Some(action) = match_action(&kb, &key) else { return };
            ev.prevent_default();

            match action.as_str() {
                "next" => {
                    if !jp_handle_reader_action("next", &key) {
                        jp_next();
                    }
                }
                "prev" => {
                    if !jp_handle_reader_action("prev", &key) {
                        jp_prev();
                    }
                }
                "toggle_flow" => {
                    let v = jp_toggle_flow();
                    let next = v.as_string().unwrap_or_else(|| "paginated".into());
                    let next: &'static str = if next == "scrolled" { "scrolled" } else { "paginated" };
                    set_flow.set(next);
                }
                "cycle_theme" => {
                    let v = jp_cycle_theme();
                    let next = v.as_string().unwrap_or_else(|| "light".into());
                    let next: &'static str = match next.as_str() {
                        "dark" => "dark",
                        "sepia" => "sepia",
                        _ => "light",
                    };
                    set_theme.set(next);
                }
                "font_up" => {
                    let v = (font_scale.get_untracked() + 0.05).min(2.0);
                    set_font_scale.set(v);
                    jp_set_font_scale(v);
                }
                "font_down" => {
                    let v = (font_scale.get_untracked() - 0.05).max(0.6);
                    set_font_scale.set(v);
                    jp_set_font_scale(v);
                }
                "toggle_settings" => set_settings_open.update(|v| *v = !*v),
                _ => {}
            }
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);
        let _ = window
            .add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
        cb.forget();
    });
    let on_font_input = move |ev: leptos::ev::Event| {
        let v: f64 = event_target_value(&ev).parse().unwrap_or(1.0);
        set_font_scale.set(v);
        jp_set_font_scale(v);
    };
    let on_line_input = move |ev: leptos::ev::Event| {
        let v: f64 = event_target_value(&ev).parse().unwrap_or(1.7);
        set_line_height.set(v);
        jp_set_line_height(v);
    };

    view! {
        <main class="reader-shell">
            <header class="reader-toolbar">
                <span class="muted">"work " {work_id}</span>
                {move || entry.get().map(|e| view! {
                    <span class="reader-title">{e.title.clone()}</span>
                    {e.author.clone().map(|a| view! {
                        <span class="muted reader-author">{a}</span>
                    })}
                })}
                {move || {
                    let s = status.get();
                    if s.is_empty() {
                        view! { <span></span> }.into_any()
                    } else {
                        view! { <span class="muted reader-status">{s}</span> }.into_any()
                    }
                }}
                <div class="reader-spacer"></div>
                <button type="button" class="reader-control" on:click=on_toggle_flow title="Toggle paginated / scrolled">
                    {move || if flow.get() == "scrolled" { "⇅" } else { "⇆" }}
                </button>
                <button type="button" class="reader-control" on:click=on_cycle_theme title="Cycle theme">
                    {move || match theme.get() {
                        "dark" => "☾",
                        "sepia" => "✶",
                        _ => "☀",
                    }}
                </button>
                <button type="button" class="reader-control" on:click=on_toggle_settings title="Settings">
                    "⚙"
                </button>
            </header>

            {move || settings_open.get().then(|| view! {
                <div class="reader-settings">
                    <label>
                        <span>"Font size"</span>
                        <input
                            type="range"
                            min="0.6"
                            max="2.0"
                            step="0.05"
                            prop:value=move || font_scale.get().to_string()
                            on:input=on_font_input
                        />
                        <span class="muted">{move || format!("{:.0}%", font_scale.get() * 100.0)}</span>
                    </label>
                    <label>
                        <span>"Line spacing"</span>
                        <input
                            type="range"
                            min="1.2"
                            max="2.4"
                            step="0.05"
                            prop:value=move || line_height.get().to_string()
                            on:input=on_line_input
                        />
                        <span class="muted">{move || format!("{:.2}", line_height.get())}</span>
                    </label>

                    <hr class="reader-settings-divider" />
                    <KeybindEditor
                        keybinds=keybinds
                        set_keybinds=set_keybinds
                        capturing=capturing
                        set_capturing=set_capturing
                    />
                </div>
            })}

            <div class="reader-stage" node_ref=stage_ref>
                {move || (flow.get() == "paginated").then(|| view! {
                    <button
                        type="button"
                        class="edge-arrow edge-prev"
                        on:click=on_prev
                        title="Previous page"
                        aria-label="Previous page"
                    >"‹"</button>
                    <button
                        type="button"
                        class="edge-arrow edge-next"
                        on:click=on_next
                        title="Next page"
                        aria-label="Next page"
                    >"›"</button>
                })}
            </div>
        </main>
    }
}

#[component]
fn KeybindEditor(
    keybinds: ReadSignal<Keybinds>,
    set_keybinds: WriteSignal<Keybinds>,
    capturing: ReadSignal<Option<String>>,
    set_capturing: WriteSignal<Option<String>>,
) -> impl IntoView {
    let on_reset = move |_| {
        set_keybinds.set(defaults());
        save_keybinds(&keybinds.get_untracked());
    };

    view! {
        <div class="reader-keybinds">
            <div class="reader-keybinds-header">
                <h3>"Keyboard shortcuts"</h3>
                <button type="button" class="reader-keybinds-reset" on:click=on_reset>"Reset"</button>
            </div>
            <p class="muted reader-keybinds-hint">
                "Click "<strong>"+"</strong>" to add a key, "<strong>"×"</strong>" to remove. Esc cancels capture."
            </p>
            <div class="reader-keybinds-rows">
                {ACTION_DEFAULTS.iter().map(|(id, label, _)| {
                    let id = id.to_string();
                    let label = (*label).to_string();
                    let id_for_add = id.clone();
                    let id_for_capture = id.clone();
                    let on_add = move |_| {
                        set_capturing.set(Some(id_for_add.clone()));
                    };
                    view! {
                        <div class="reader-keybinds-row">
                            <span class="reader-keybinds-label">{label}</span>
                            <span class="reader-keybinds-keys">
                                {move || {
                                    let id = id.clone();
                                    let kb = keybinds.get();
                                    let keys = kb.get(&id).cloned().unwrap_or_default();
                                    keys.into_iter().map(|key| {
                                        let key_for_remove = key.clone();
                                        let id_for_remove = id.clone();
                                        let on_remove = move |_| {
                                            set_keybinds.update(|kb| {
                                                if let Some(v) = kb.get_mut(&id_for_remove) {
                                                    v.retain(|k| k != &key_for_remove);
                                                }
                                            });
                                            save_keybinds(&keybinds.get_untracked());
                                        };
                                        view! {
                                            <span class="kbd-chip">
                                                <kbd>{label_key(&key)}</kbd>
                                                <button
                                                    type="button"
                                                    class="kbd-chip-remove"
                                                    title="Remove"
                                                    on:click=on_remove
                                                >"×"</button>
                                            </span>
                                        }
                                    }).collect_view()
                                }}
                                {move || {
                                    let active = capturing.get().as_deref() == Some(id_for_capture.as_str());
                                    if active {
                                        view! {
                                            <span class="kbd-chip kbd-chip-capturing">
                                                "Press a key…"
                                            </span>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <button
                                                type="button"
                                                class="kbd-chip-add"
                                                on:click=on_add.clone()
                                                title="Add binding"
                                            >"+"</button>
                                        }.into_any()
                                    }
                                }}
                            </span>
                        </div>
                    }
                }).collect_view()}
            </div>
        </div>
    }
}

fn make_blob(bytes: &JsValue) -> Result<web_sys::Blob, String> {
    let arr = js_sys::Array::new();
    arr.push(bytes);
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/epub+zip");
    web_sys::Blob::new_with_buffer_source_sequence_and_options(&arr, &opts)
        .map_err(|e| format!("Blob::new failed: {}", stringify(&e)))
}

fn stringify(v: &JsValue) -> String {
    v.as_string()
        .or_else(|| js_sys::JSON::stringify(v).ok().map(|s| s.into()))
        .unwrap_or_else(|| "unknown error".into())
}
