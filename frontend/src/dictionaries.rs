//! Phase 5 subpiece 1 — "Dictionaries" section in the main window.
//!
//! Folder-picker triggers batch import via the Tauri command
//! `import_dictionary_folder`; progress events stream in via the
//! `dictionary-import-progress` event; the list below refreshes
//! after each successful import. Each row has a Delete button that
//! cascades to its term/kanji/meta/tag rows.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], catch)]
    async fn listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> Result<JsValue, JsValue>;

    // Tauri dialog plugin lives under window.__TAURI__.dialog when
    // withGlobalTauri is on.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "dialog"], catch)]
    async fn open(options: JsValue) -> Result<JsValue, JsValue>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Dictionary {
    id: i64,
    name: String,
    #[serde(default)]
    revision: Option<String>,
    format_version: i32,
    priority: i32,
    enabled: bool,
    imported_at: i64,
    term_count: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct ProgressEvent {
    current: usize,
    total: usize,
    path: String,
    status: String,
}

fn stringify_err(v: JsValue) -> String {
    v.as_string()
        .or_else(|| js_sys::JSON::stringify(&v).ok().map(|s| s.into()))
        .unwrap_or_else(|| "unknown error".into())
}

#[component]
pub fn DictionariesPanel() -> impl IntoView {
    let (dicts, set_dicts) = signal::<Vec<Dictionary>>(Vec::new());
    let (busy, set_busy) = signal::<bool>(false);
    let (progress, set_progress) = signal::<Option<String>>(None);
    let (banner, set_banner) = signal::<Option<String>>(None);

    let refresh = move || {
        spawn_local(async move {
            match invoke("list_dictionaries", JsValue::from_str("{}")).await {
                Ok(v) => {
                    if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<Dictionary>>(v) {
                        set_dicts.set(rows);
                    }
                }
                Err(e) => set_banner.set(Some(format!("list: {}", stringify_err(e)))),
            }
        });
    };

    refresh();

    // Subscribe to import progress for the lifetime of the page.
    Effect::new(move |_| {
        let cb = Closure::wrap(Box::new(move |e: JsValue| {
            // Tauri's listen() passes an event object { event, id,
            // payload }. We only need the payload.
            let payload =
                js_sys::Reflect::get(&e, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
            if let Ok(p) = serde_wasm_bindgen::from_value::<ProgressEvent>(payload) {
                let name = std::path::Path::new(&p.path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&p.path)
                    .to_string();
                set_progress.set(Some(format!(
                    "[{}/{}] {} — {}",
                    p.current, p.total, p.status, name
                )));
            }
        }) as Box<dyn FnMut(JsValue)>);
        spawn_local(async move {
            let _ = listen("dictionary-import-progress", &cb).await;
            // Leak the closure so it stays alive for the page's
            // lifetime. The listen() promise resolves with an
            // unsubscribe fn that we deliberately drop.
            cb.forget();
        });
    });

    let on_import_folder = move |_| {
        set_busy.set(true);
        set_banner.set(None);
        spawn_local(async move {
            // 1) Open the OS folder picker.
            let opts = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &opts,
                &JsValue::from_str("directory"),
                &JsValue::from_bool(true),
            );
            let _ = js_sys::Reflect::set(
                &opts,
                &JsValue::from_str("multiple"),
                &JsValue::from_bool(false),
            );
            let picked = match open(opts.into()).await {
                Ok(v) => v,
                Err(e) => {
                    set_banner.set(Some(format!("dialog: {}", stringify_err(e))));
                    set_busy.set(false);
                    return;
                }
            };
            let Some(path) = picked.as_string() else {
                // User cancelled — dialog returns null.
                set_busy.set(false);
                return;
            };

            // 2) Kick off the batch import.
            set_progress.set(Some("Starting import…".into()));
            let args = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(&path));
            let outcomes = invoke("import_dictionary_folder", args.into()).await;
            match outcomes {
                Ok(v) => {
                    let arr = js_sys::Array::from(&v);
                    let total = arr.length() as usize;
                    let mut imported = 0usize;
                    let mut skipped = 0usize;
                    let mut failed = 0usize;
                    for i in 0..arr.length() {
                        let entry = arr.get(i);
                        let kind = js_sys::Reflect::get(&entry, &JsValue::from_str("kind"))
                            .ok()
                            .and_then(|v| v.as_string())
                            .unwrap_or_default();
                        match kind.as_str() {
                            "imported" => imported += 1,
                            "skipped" => skipped += 1,
                            "failed" => failed += 1,
                            _ => {}
                        }
                    }
                    set_banner.set(Some(format!(
                        "{total} processed: {imported} imported, {skipped} skipped, {failed} failed."
                    )));
                    set_progress.set(None);
                    refresh();
                }
                Err(e) => {
                    set_banner.set(Some(format!("import: {}", stringify_err(e))));
                    set_progress.set(None);
                }
            }
            set_busy.set(false);
        });
    };

    view! {
        <section class="dictionaries">
            <details open>
                <summary><h2 style="display:inline">"Dictionaries"</h2></summary>
                <div class="row">
                    <button
                        type="button"
                        on:click=on_import_folder
                        prop:disabled=move || busy.get()
                    >
                        {move || if busy.get() { "Importing…" } else { "Import folder…" }}
                    </button>
                    {move || progress.get().map(|p| view! { <span class="muted">{p}</span> })}
                </div>
                {move || banner.get().map(|b| view! { <div class="banner">{b}</div> })}
                {move || {
                    let rows = dicts.get();
                    if rows.is_empty() {
                        view! {
                            <p class="muted">"No dictionaries imported yet. Click \"Import folder…\" to pick a directory of Yomitan .zip files."</p>
                        }.into_any()
                    } else {
                        view! {
                            <table class="dict-table">
                                <thead>
                                    <tr>
                                        <th>"Name"</th>
                                        <th>"Format"</th>
                                        <th>"Revision"</th>
                                        <th>"Terms"</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {rows.into_iter().map(|d| {
                                        let id = d.id;
                                        let name = d.name.clone();
                                        let rev = d.revision.clone().unwrap_or_default();
                                        let fmt = d.format_version;
                                        let tc = d.term_count;
                                        let refresh = refresh;
                                        let on_delete = move |_| {
                                            let n = name.clone();
                                            spawn_local(async move {
                                                let args = js_sys::Object::new();
                                                let _ = js_sys::Reflect::set(
                                                    &args,
                                                    &JsValue::from_str("id"),
                                                    &JsValue::from_f64(id as f64),
                                                );
                                                match invoke("delete_dictionary", args.into()).await {
                                                    Ok(_) => refresh(),
                                                    Err(e) => web_sys::console::warn_1(
                                                        &format!("delete {n}: {}", stringify_err(e)).into(),
                                                    ),
                                                }
                                            });
                                        };
                                        view! {
                                            <tr>
                                                <td>{d.name.clone()}</td>
                                                <td>{format!("v{fmt}")}</td>
                                                <td class="muted">{rev}</td>
                                                <td>{tc}</td>
                                                <td><button type="button" on:click=on_delete>"Delete"</button></td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        }.into_any()
                    }
                }}
            </details>
        </section>
    }
}
