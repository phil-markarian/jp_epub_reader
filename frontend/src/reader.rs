//! Phase 3 reader window. Renders an EPUB through foliate-js inside a
//! per-work webview window. Phase 5 will add dictionary lookup over
//! click events captured here.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
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

    #[wasm_bindgen(js_namespace = ["window", "__JP_READER"], js_name = "wheelScroll")]
    fn jp_wheel_scroll(dx: f64, dy: f64);

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

#[component]
pub fn ReaderApp(work_id: u32) -> impl IntoView {
    let (entry, set_entry) = signal::<Option<LibraryEntry>>(None);
    let (status, set_status) = signal::<String>("Loading…".into());
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
            match jp_mount(stage_el, blob, JsValue::NULL, JsValue::NULL).await {
                Ok(_) => set_status.set(String::new()),
                Err(v) => set_status.set(format!("mount: {}", stringify(&v))),
            }
        });
    });

    let (theme, set_theme) = signal::<&'static str>("light");
    let (flow, set_flow) = signal::<&'static str>("paginated");
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

    // Debounce wheel events so a single trackpad scroll doesn't fly
    // through 4 pages.
    let last_wheel_ms = StoredValue::new(0.0_f64);
    let on_wheel = move |ev: leptos::ev::WheelEvent| {
        if flow.get() == "scrolled" {
            // Translate vertical wheel to horizontal scroll for tategaki.
            ev.prevent_default();
            jp_wheel_scroll(ev.delta_x(), ev.delta_y());
            return;
        }
        let now = js_sys::Date::now();
        if now - last_wheel_ms.get_value() < 250.0 {
            return;
        }
        last_wheel_ms.set_value(now);
        ev.prevent_default();
        let dy = ev.delta_y() + ev.delta_x();
        if dy > 0.0 { jp_next(); } else if dy < 0.0 { jp_prev(); }
    };

    let (settings_open, set_settings_open) = signal::<bool>(false);
    let (font_scale, set_font_scale) = signal::<f64>(1.0);
    let (line_height, set_line_height) = signal::<f64>(1.7);

    let on_toggle_settings = move |_| set_settings_open.update(|v| *v = !*v);
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
                </div>
            })}

            <div class="reader-stage" node_ref=stage_ref on:wheel=on_wheel>
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
