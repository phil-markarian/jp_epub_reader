//! Phase 3 reader window. Phase 3 just wires up the window + a stub
//! page. Phase 3b plugs in foliate-js to actually render the EPUB.
//! Phase 5 adds dictionary lookup over click events.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen::closure::Closure;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
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
    let (error, set_error) = signal::<Option<String>>(None);

    spawn_local(async move {
        let v = serde_wasm_bindgen::to_value(&WorkArgs { work_id }).unwrap();
        match invoke("get_library_entry", v).await {
            Ok(v) => match serde_wasm_bindgen::from_value::<LibraryEntry>(v) {
                Ok(e) => set_entry.set(Some(e)),
                Err(e) => set_error.set(Some(format!("decode: {e}"))),
            },
            Err(v) => set_error.set(Some(stringify(&v))),
        }
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
            </header>

            {move || error.get().map(|msg| view! {
                <div class="banner">{msg}</div>
            })}

            <section class="reader-stage">
                {move || match entry.get() {
                    None => view! { <p class="muted">"Loading…"</p> }.into_any(),
                    Some(e) => view! {
                        <p class="muted">
                            "Phase 3a placeholder. EPUB will render here once foliate-js is wired in."
                        </p>
                        <p class="muted">
                            "EPUB path: " <code>{e.epub_path}</code>
                        </p>
                    }.into_any()
                }}
            </section>
        </main>
    }
}

fn stringify(v: &JsValue) -> String {
    v.as_string()
        .or_else(|| js_sys::JSON::stringify(v).ok().map(|s| s.into()))
        .unwrap_or_else(|| "unknown error".into())
}

// Suppress unused-import warning until we add subscriptions.
#[allow(dead_code)]
fn _force_use(_: Closure<dyn Fn()>) {}
