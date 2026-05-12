//! Phase 3 reader window. Renders an EPUB through foliate-js inside a
//! per-work webview window. Phase 5 will add dictionary lookup over
//! click events captured here.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

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

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "getCurrentLocation")]
    fn jp_get_current_location() -> JsValue;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "goToBookmark")]
    fn jp_go_to_bookmark(target: JsValue);

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "resolveChapterForSection")]
    fn jp_resolve_chapter_for_section(section_index: u32) -> JsValue;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "getChapterList")]
    fn jp_get_chapter_list() -> JsValue;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "goToChapter")]
    fn jp_go_to_chapter(
        section_index: u32,
        chapter_id: JsValue,
        index_in_section: JsValue,
    );

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "getChapterPosition")]
    fn jp_get_chapter_position(section_index: u32) -> JsValue;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "getCurrentChapterIndex")]
    fn jp_get_current_chapter_index() -> JsValue;

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "setChapterChangeCallback")]
    fn jp_set_chapter_change_callback(cb: JsValue);

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "pinCurrentChapter")]
    fn jp_pin_current_chapter(idx: u32, ms: u32);
}

#[derive(Clone, Debug, Deserialize)]
struct ChapterPosition {
    current: u32,
    total: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChapterEntry {
    #[serde(rename = "sectionIndex")]
    section_index: u32,
    level: u32,
    label: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "indexInSection")]
    index_in_section: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CurrentLocation {
    cfi: Option<String>,
    #[serde(rename = "sectionIndex")]
    section_index: Option<u32>,
    fraction: Option<f64>,
    chapter: Option<String>,
    #[serde(rename = "chapterIndex")]
    chapter_index: Option<u32>,
    #[serde(rename = "chapterTotal")]
    chapter_total: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Bookmark {
    id: i64,
    work_id: u32,
    cfi: Option<String>,
    section_index: Option<u32>,
    fraction: Option<f64>,
    chapter: Option<String>,
    #[serde(default)]
    chapter_index: Option<u32>,
    #[serde(default)]
    chapter_total: Option<u32>,
    note: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddBookmarkArgs {
    work_id: u32,
    cfi: Option<String>,
    section_index: Option<u32>,
    fraction: Option<f64>,
    chapter: Option<String>,
    chapter_index: Option<u32>,
    chapter_total: Option<u32>,
    note: Option<String>,
}

#[derive(Serialize)]
struct ListBookmarksArgs {
    #[serde(rename = "workId")]
    work_id: u32,
}

#[derive(Serialize)]
struct UpdateBookmarkNoteArgs<'a> {
    id: i64,
    note: &'a str,
}

#[derive(Serialize)]
struct DeleteBookmarkArgs {
    id: i64,
}

async fn invoke_typed<T: serde::Serialize, R: serde::de::DeserializeOwned>(
    cmd: &str,
    args: &T,
) -> Result<R, String> {
    let v = serde_wasm_bindgen::to_value(args).unwrap();
    match invoke(cmd, v).await {
        Ok(out) => serde_wasm_bindgen::from_value::<R>(out)
            .map_err(|e| format!("decode {cmd}: {e}")),
        Err(e) => Err(stringify(&e)),
    }
}

async fn invoke_unit<T: serde::Serialize>(cmd: &str, args: &T) -> Result<(), String> {
    let v = serde_wasm_bindgen::to_value(args).unwrap();
    invoke(cmd, v).await.map(|_| ()).map_err(|e| stringify(&e))
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
    let (book_fraction, set_book_fraction) = signal::<Option<f64>>(None);
    // Active-chapter tracking lives up here so the relocate callback
    // (built inside the mount Effect below) can capture the
    // WriteSignal by move. The ChapterDrawer reads `current_chapter_idx`
    // to highlight the matching row.
    let (current_chapter_idx, set_current_chapter_idx) = signal::<Option<usize>>(None);
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
            // Build a relocate callback that pushes the book-wide
            // progress fraction into the bottom-left badge. Foliate
            // populates detail.fraction via SectionProgress, which is
            // the position across the whole book (0..1) — not the
            // section.
            let relocate_cb = Closure::wrap(Box::new(move |detail: JsValue| {
                let f = js_sys::Reflect::get(&detail, &JsValue::from_str("fraction"))
                    .ok()
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite());
                if let Some(f) = f {
                    set_book_fraction.set(Some(f));
                }
                // Keep the chapter drawer's highlighted row in sync
                // even while it's already open.
                let cur_raw = jp_get_current_chapter_index();
                if let Some(idx) = cur_raw.as_f64() {
                    if idx.is_finite() && idx >= 0.0 {
                        set_current_chapter_idx.set(Some(idx as usize));
                        return;
                    }
                }
                set_current_chapter_idx.set(None);
            }) as Box<dyn FnMut(JsValue)>);
            let relocate_js: JsValue = relocate_cb.as_ref().clone();
            // Leak so the listener keeps firing for the window's
            // lifetime — matches the pattern used by the keyboard
            // closure below.
            relocate_cb.forget();

            // Live chapter tracker — fired by the scroll-driven path
            // in reader-init.js, independent of Foliate's 250ms
            // relocate debounce. Keeps the chapter drawer highlight
            // glued to the user's actual scroll position.
            let chapter_cb = Closure::wrap(Box::new(move |idx: JsValue| {
                if let Some(n) = idx.as_f64() {
                    if n.is_finite() && n >= 0.0 {
                        set_current_chapter_idx.set(Some(n as usize));
                        return;
                    }
                }
                set_current_chapter_idx.set(None);
            }) as Box<dyn FnMut(JsValue)>);
            let chapter_js: JsValue = chapter_cb.as_ref().clone();
            chapter_cb.forget();
            jp_set_chapter_change_callback(chapter_js);

            match jp_mount(stage_el, blob, work_id, relocate_js, JsValue::NULL).await {
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
    // All three sidebar-panel open flags up here so the toggle
    // handlers below can mutex them.
    let (settings_open, set_settings_open) = signal::<bool>(false);
    let (bookmarks_open, set_bookmarks_open) = signal::<bool>(false);
    let (chapters_open, set_chapters_open) = signal::<bool>(false);

    let (font_scale, set_font_scale) = signal::<f64>(1.0);
    let (line_height, set_line_height) = signal::<f64>(1.7);

    let on_toggle_settings = move |_| {
        let next = !settings_open.get_untracked();
        set_settings_open.set(next);
        if next {
            // The three panels share the same sidebar slot.
            set_bookmarks_open.set(false);
            set_chapters_open.set(false);
        }
    };

    // Bookmarks data
    let (bookmarks, set_bookmarks) = signal::<Vec<Bookmark>>(Vec::new());
    let (bookmark_error, set_bookmark_error) = signal::<Option<String>>(None);

    // Chapter list (derived from inline AozoraEpub3 markers cached at
    // book open). Refreshed each time the drawer is opened because
    // sections load lazily during paginated reading and the live
    // cache picks up newer content.
    let (chapters, set_chapters) = signal::<Vec<ChapterEntry>>(Vec::new());

    let refresh_chapters = move || {
        let raw = jp_get_chapter_list();
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ChapterEntry>>(raw) {
            set_chapters.set(list);
        }
        // Same call surface — refresh "what chapter am I on" too, so
        // the drawer marks the current row when it opens.
        let cur_raw = jp_get_current_chapter_index();
        if let Some(idx) = cur_raw.as_f64() {
            if idx.is_finite() && idx >= 0.0 {
                set_current_chapter_idx.set(Some(idx as usize));
            }
        } else {
            set_current_chapter_idx.set(None);
        }
    };

    let on_toggle_chapters = move |_| {
        let next = !chapters_open.get_untracked();
        set_chapters_open.set(next);
        if next {
            refresh_chapters();
            // The three panels share the same sidebar slot.
            set_bookmarks_open.set(false);
            set_settings_open.set(false);
        }
    };

    let refresh_bookmarks = move || {
        spawn_local(async move {
            match invoke_typed::<_, Vec<Bookmark>>(
                "list_bookmarks",
                &ListBookmarksArgs { work_id },
            )
            .await
            {
                Ok(rows) => set_bookmarks.set(rows),
                Err(e) => set_bookmark_error.set(Some(format!("list: {e}"))),
            }
        });
    };

    let on_toggle_bookmarks = move |_| {
        let next = !bookmarks_open.get_untracked();
        set_bookmarks_open.set(next);
        if next {
            // The three panels share the same sidebar slot.
            set_chapters_open.set(false);
            set_settings_open.set(false);
            refresh_bookmarks();
        }
    };

    let on_add_bookmark = move |_| {
        let raw = jp_get_current_location();
        let loc: CurrentLocation = serde_wasm_bindgen::from_value(raw).unwrap_or_default();
        set_bookmark_error.set(None);
        spawn_local(async move {
            let args = AddBookmarkArgs {
                work_id,
                cfi: loc.cfi,
                section_index: loc.section_index,
                fraction: loc.fraction,
                chapter: loc.chapter,
                chapter_index: loc.chapter_index,
                chapter_total: loc.chapter_total,
                note: None,
            };
            match invoke_typed::<_, Bookmark>("add_bookmark", &args).await {
                Ok(_) => {
                    set_chapters_open.set(false);
                    set_settings_open.set(false);
                    set_bookmarks_open.set(true);
                    refresh_bookmarks();
                }
                Err(e) => set_bookmark_error.set(Some(format!("add: {e}"))),
            }
        });
    };

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
            // Let editable fields swallow keys themselves so the user
            // can type their bookmark note (j/k/space/arrows etc. would
            // otherwise be intercepted as nav shortcuts).
            if is_editable_target(ev.target().as_ref()) {
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
                <div class="reader-toolbar-left">
                    <span class="muted">"work " {work_id}</span>
                    {move || {
                        let s = status.get();
                        if s.is_empty() {
                            view! { <span></span> }.into_any()
                        } else {
                            view! { <span class="muted reader-status">{s}</span> }.into_any()
                        }
                    }}
                </div>
                <div class="reader-toolbar-center">
                    {move || entry.get().map(|e| {
                        let body = match e.author.as_ref() {
                            Some(a) => format!("{a}: {}", e.title),
                            None => e.title.clone(),
                        };
                        view! { <span class="reader-title">{body}</span> }
                    })}
                </div>
                <div class="reader-toolbar-right">
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
                    <button type="button" class="reader-control" on:click=on_toggle_chapters title="Show chapters">
                        "📑"
                    </button>
                    <button type="button" class="reader-control" on:click=on_add_bookmark title="Bookmark this page">
                        "＋🔖"
                    </button>
                    <button type="button" class="reader-control" on:click=on_toggle_bookmarks title="Show bookmarks">
                        "🔖"
                    </button>
                    <button type="button" class="reader-control" on:click=on_toggle_settings title="Settings">
                        "⚙"
                    </button>
                </div>
            </header>

            <div class="reader-body">
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

                {move || {
                    let any_open = settings_open.get()
                        || bookmarks_open.get()
                        || chapters_open.get();
                    if !any_open {
                        return view! { <span></span> }.into_any();
                    }
                    view! {
                        <aside class="reader-sidebar">
                            {move || settings_open.get().then(|| view! {
                                <div class="reader-settings">
                                    <header class="reader-sidebar-header">
                                        <h3>"Settings"</h3>
                                        <button type="button" class="reader-control"
                                            on:click=move |_| set_settings_open.set(false)
                                            title="Close">"×"</button>
                                    </header>
                                    <div class="reader-sidebar-body">
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
                                </div>
                            })}

                            {move || bookmarks_open.get().then(|| view! {
                                <BookmarkDrawer
                                    bookmarks=bookmarks
                                    error=bookmark_error
                                    set_error=set_bookmark_error
                                    refresh=refresh_bookmarks
                                    on_close=move |_| set_bookmarks_open.set(false)
                                    on_add=on_add_bookmark
                                />
                            })}

                            {move || chapters_open.get().then(|| view! {
                                <ChapterDrawer
                                    chapters=chapters
                                    current_idx=current_chapter_idx
                                    set_current_idx=set_current_chapter_idx
                                    on_close=move |_| set_chapters_open.set(false)
                                />
                            })}
                        </aside>
                    }.into_any()
                }}
            </div>

            <div class="reader-progress" aria-hidden=move || book_fraction.get().is_none().to_string()>
                {move || book_fraction.get()
                    .map(|f| {
                        let pct = ((f * 100.0).round() as i32).clamp(0, 100);
                        format!("{pct}%")
                    })
                    .unwrap_or_else(|| "—".to_string())
                }
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

#[component]
fn BookmarkDrawer(
    bookmarks: ReadSignal<Vec<Bookmark>>,
    error: ReadSignal<Option<String>>,
    set_error: WriteSignal<Option<String>>,
    refresh: impl Fn() + Copy + 'static + Send + Sync,
    on_close: impl Fn(leptos::ev::MouseEvent) + 'static,
    on_add: impl Fn(leptos::ev::MouseEvent) + Copy + 'static,
) -> impl IntoView {
    view! {
        <aside class="bookmark-drawer">
            <header class="bookmark-drawer-header">
                <h3>"Bookmarks"</h3>
                <button type="button" class="reader-control" on:click=on_add title="Bookmark current page">
                    "＋"
                </button>
                <button type="button" class="reader-control" on:click=on_close title="Close">
                    "×"
                </button>
            </header>
            {move || error.get().map(|msg| view! {
                <div class="bookmark-drawer-error">{msg}</div>
            })}
            {move || {
                let rows = bookmarks.get();
                if rows.is_empty() {
                    view! {
                        <p class="muted bookmark-drawer-empty">
                            "No bookmarks yet. Click "<strong>"＋🔖"</strong>" in the toolbar to bookmark the current page."
                        </p>
                    }.into_any()
                } else {
                    view! {
                        <ul class="bookmark-list">
                            {rows.into_iter().map(|b| view! {
                                <li>
                                    <BookmarkRow
                                        bookmark=b
                                        refresh=refresh
                                        set_error=set_error
                                    />
                                </li>
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
        </aside>
    }
}

#[component]
fn ChapterDrawer(
    chapters: ReadSignal<Vec<ChapterEntry>>,
    current_idx: ReadSignal<Option<usize>>,
    set_current_idx: WriteSignal<Option<usize>>,
    on_close: impl Fn(leptos::ev::MouseEvent) + 'static,
) -> impl IntoView {
    let list_ref: NodeRef<leptos::html::Ul> = NodeRef::new();
    let indicator_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    // First Effect run positions the indicator without animation; we
    // defer arming the CSS transition by one frame so subsequent
    // updates animate. StoredValue so the flag survives across the
    // Effect's reactive re-runs without being a tracked dependency.
    let armed = StoredValue::new(false);

    // Position the sliding highlight + scroll the current row into
    // view whenever the current chapter or the row list changes.
    Effect::new(move |_| {
        let _ = chapters.get();
        let Some(ul) = list_ref.get() else { return };
        let Some(indicator) = indicator_ref.get() else { return };
        let indicator_el: web_sys::HtmlElement = (*indicator).clone().into();
        // When current_idx briefly goes None (the relocate-derived
        // path returns null at the very top of the first section
        // before any chapter heading, and during fast navigation),
        // leave the indicator parked at its last position instead of
        // fading it out. Avoids the off/on flicker at book boundaries.
        let Some(idx) = current_idx.get() else { return };
        let row_el = ul
            .query_selector(&format!(".chapter-row[data-idx=\"{idx}\"]"))
            .ok()
            .flatten();
        let Some(row_el) = row_el else { return };
        let Ok(row_html) = row_el.dyn_into::<web_sys::HtmlElement>() else { return };
        let top = row_html.offset_top();
        let height = row_html.offset_height();
        let style = indicator_el.style();
        let _ = style.set_property(
            "transform",
            &format!("translateY({top}px)"),
        );
        let _ = style.set_property("height", &format!("{height}px"));
        let _ = style.set_property("opacity", "1");

        // Auto-scroll the current row into view.
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_behavior(web_sys::ScrollBehavior::Smooth);
        opts.set_block(web_sys::ScrollLogicalPosition::Center);
        row_html.scroll_into_view_with_scroll_into_view_options(&opts);

        // Arm the CSS transition after the first paint so the very
        // first positioning doesn't slide from (0, 0).
        if !armed.get_value() {
            armed.set_value(true);
            let target = indicator_el.clone();
            let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                let _ = target.set_attribute("data-armed", "true");
            });
            if let Some(win) = web_sys::window() {
                let _ = win.request_animation_frame(cb.as_ref().unchecked_ref());
            }
        }
    });

    view! {
        <aside class="bookmark-drawer chapter-drawer">
            <header class="bookmark-drawer-header">
                <h3>"Chapters"</h3>
                <button type="button" class="reader-control" on:click=on_close title="Close">
                    "×"
                </button>
            </header>
            {move || {
                let rows = chapters.get();
                if rows.is_empty() {
                    view! {
                        <p class="muted bookmark-drawer-empty">
                            "No chapter markers found in this book."
                        </p>
                    }.into_any()
                } else {
                    let current = current_idx.get();
                    view! {
                        <ul class="chapter-list" node_ref=list_ref>
                            <div class="chapter-highlight" node_ref=indicator_ref></div>
                            {rows.into_iter().enumerate().map(|(i, c)| {
                                let level = c.level.clamp(1, 3);
                                let section_index = c.section_index;
                                let id_opt = c.id.clone();
                                let label = c.label.clone();
                                let index_in_section = c.index_in_section;
                                let is_current = current == Some(i);
                                let row_class = if is_current {
                                    format!("chapter-row chapter-level-{level} is-current")
                                } else {
                                    format!("chapter-row chapter-level-{level}")
                                };
                                let on_click = move |_| {
                                    let id_js = match id_opt.clone() {
                                        Some(s) => JsValue::from_str(&s),
                                        None => JsValue::NULL,
                                    };
                                    let idx_js = match index_in_section {
                                        Some(j) => JsValue::from_f64(j as f64),
                                        None => JsValue::NULL,
                                    };
                                    // Move the highlight optimistically and
                                    // pin the JS-side chapter index for a
                                    // beat so the relocate-debounced
                                    // recompute (which can resolve to the
                                    // chapter PAST the heading once the goTo
                                    // scroll lands) doesn't immediately
                                    // override the user's pick.
                                    set_current_idx.set(Some(i));
                                    jp_pin_current_chapter(i as u32, 700);
                                    jp_go_to_chapter(section_index, id_js, idx_js);
                                };
                                view! {
                                    <li class=row_class data-idx=i>
                                        <button
                                            type="button"
                                            class="chapter-jump"
                                            on:click=on_click
                                            title="Jump to chapter"
                                            aria-current=if is_current { "true" } else { "false" }
                                        >
                                            <span class="chapter-label">{label}</span>
                                            <span class="muted chapter-section">
                                                {format!("§{section_index}")}
                                            </span>
                                        </button>
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
        </aside>
    }
}

#[component]
fn BookmarkRow(
    bookmark: Bookmark,
    refresh: impl Fn() + Copy + 'static + Send + Sync,
    set_error: WriteSignal<Option<String>>,
) -> impl IntoView {
    let id = bookmark.id;
    let stored_chapter = bookmark.chapter.clone().unwrap_or_default();
    // Live-resolve from the chapter cache when the stored label is
    // empty (older bookmarks captured before the inline-markup
    // resolver landed). The stored value still wins when present.
    let chapter_for_display = if !stored_chapter.is_empty() {
        stored_chapter
    } else if let Some(idx) = bookmark.section_index {
        match jp_resolve_chapter_for_section(idx).as_string() {
            Some(s) if !s.is_empty() => s,
            _ => format!("Section {idx}"),
        }
    } else {
        "Unknown chapter".into()
    };
    // "Chapter X of Y". Prefer the precise flat chapter index stored
    // at bookmark creation; fall back to the section-derived estimate
    // for older bookmarks that don't have it.
    let position_text: Option<String> = match (bookmark.chapter_index, bookmark.chapter_total) {
        (Some(i), Some(total)) if total > 0 => {
            Some(format!("Chapter {} of {}", i + 1, total))
        }
        _ => bookmark.section_index.and_then(|idx| {
            let raw = jp_get_chapter_position(idx);
            serde_wasm_bindgen::from_value::<ChapterPosition>(raw)
                .ok()
                .map(|p| format!("Chapter {} of {}", p.current, p.total))
        }),
    };
    let fraction = bookmark.fraction.unwrap_or(0.0);
    let progress_label = format!("{:.0}%", (fraction * 100.0).clamp(0.0, 100.0));
    let cfi = bookmark.cfi.clone();
    let initial_note = bookmark.note.clone();

    let (note, set_note) = signal::<String>(initial_note);
    let (saving, set_saving) = signal::<bool>(false);
    let (confirm_delete, set_confirm_delete) = signal::<bool>(false);

    let on_go = move |_| {
        if let Some(cfi) = cfi.clone() {
            jp_go_to_bookmark(JsValue::from_str(&cfi));
        }
    };

    let on_note_input = move |ev: leptos::ev::Event| {
        let v = event_target_value(&ev);
        set_note.set(v);
    };

    let on_note_blur = move |_| {
        let value = note.get_untracked();
        set_saving.set(true);
        spawn_local(async move {
            match invoke_unit(
                "update_bookmark_note",
                &UpdateBookmarkNoteArgs { id, note: &value },
            )
            .await
            {
                Ok(_) => {}
                Err(e) => set_error.set(Some(format!("save note: {e}"))),
            }
            set_saving.set(false);
        });
    };

    let on_delete = move |_| {
        if !confirm_delete.get_untracked() {
            set_confirm_delete.set(true);
            return;
        }
        set_confirm_delete.set(false);
        spawn_local(async move {
            match invoke_unit("delete_bookmark", &DeleteBookmarkArgs { id }).await {
                Ok(_) => refresh(),
                Err(e) => set_error.set(Some(format!("delete: {e}"))),
            }
        });
    };

    view! {
        <article class="bookmark-row">
            <div class="bookmark-row-head">
                <button type="button" class="bookmark-jump" on:click=on_go title="Jump to bookmark">
                    <span class="bookmark-chapter">{chapter_for_display}</span>
                    {position_text.map(|t| view! { <span class="muted bookmark-position">{t}</span> })}
                    <span class="muted bookmark-progress">{progress_label}</span>
                </button>
                <button
                    type="button"
                    class="bookmark-delete"
                    on:click=on_delete
                    title=move || if confirm_delete.get() { "Click again to confirm" } else { "Remove bookmark" }.to_string()
                >
                    {move || if confirm_delete.get() { "Confirm?" } else { "×" }}
                </button>
            </div>
            <textarea
                class="bookmark-note"
                placeholder="Add a note…"
                prop:value=move || note.get()
                on:input=on_note_input
                on:blur=on_note_blur
            ></textarea>
            {move || saving.get().then(|| view! { <span class="muted bookmark-saving">"saving…"</span> })}
        </article>
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

fn is_editable_target(target: Option<&web_sys::EventTarget>) -> bool {
    let Some(target) = target else { return false };
    if let Some(el) = target.dyn_ref::<web_sys::HtmlElement>() {
        if el.is_content_editable() {
            return true;
        }
        let tag = el.tag_name().to_ascii_uppercase();
        if tag == "TEXTAREA" || tag == "INPUT" {
            return true;
        }
    }
    false
}

fn stringify(v: &JsValue) -> String {
    v.as_string()
        .or_else(|| js_sys::JSON::stringify(v).ok().map(|s| s.into()))
        .unwrap_or_else(|| "unknown error".into())
}
