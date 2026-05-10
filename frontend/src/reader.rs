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
            </header>

            <div class="reader-stage" node_ref=stage_ref></div>
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
