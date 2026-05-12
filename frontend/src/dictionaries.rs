//! Phase 5 subpiece 1 — "Dictionaries" section in the main window.
//!
//! Two-step import flow:
//!   1. User picks a folder. We scan it recursively, read each zip's
//!      index.json, and present a checklist of dictionaries found
//!      (with status badges for already-imported / unsupported /
//!      broken).
//!   2. User unchecks anything they don't want, hits "Import
//!      selected"; we kick off the actual term-bank import only for
//!      the chosen subset. Progress events stream in via the
//!      `dictionary-import-progress` event.

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
#[serde(rename_all = "camelCase")]
struct DictPreview {
    path: String,
    name: Option<String>,
    // revision and error come back from the backend but we don't
    // surface them in the current UI; keep the fields decoded but
    // unread so future polish can pick them up without a wire change.
    #[serde(default)]
    #[allow(dead_code)]
    revision: Option<String>,
    format_version: Option<i32>,
    status: String,
    #[serde(default)]
    #[allow(dead_code)]
    error: Option<String>,
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

fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Per-zip status displayed in the import queue while a batch runs.
#[derive(Clone, Debug, PartialEq, Eq)]
enum QueueStatus {
    Queued,
    Running,
    Imported,
    Skipped,
    Failed,
}

impl QueueStatus {
    fn css_class(&self) -> &'static str {
        match self {
            QueueStatus::Queued => "queue-queued",
            QueueStatus::Running => "queue-running",
            QueueStatus::Imported => "queue-imported",
            QueueStatus::Skipped => "queue-skipped",
            QueueStatus::Failed => "queue-failed",
        }
    }
    fn label(&self) -> &'static str {
        match self {
            QueueStatus::Queued => "Queued",
            QueueStatus::Running => "Importing…",
            QueueStatus::Imported => "Imported",
            QueueStatus::Skipped => "Skipped",
            QueueStatus::Failed => "Failed",
        }
    }
}

#[component]
pub fn DictionariesPanel() -> impl IntoView {
    let (dicts, set_dicts) = signal::<Vec<Dictionary>>(Vec::new());
    let (preview, set_preview) = signal::<Vec<DictPreview>>(Vec::new());
    // Set of paths the user has selected from the preview list.
    let (selected, set_selected) = signal::<std::collections::HashSet<String>>(
        std::collections::HashSet::new(),
    );
    // Visual queue: maps each selected path to its current import
    // status during/after a batch run. Empty between batches.
    let (queue, set_queue) =
        signal::<std::collections::BTreeMap<String, QueueStatus>>(
            std::collections::BTreeMap::new(),
        );
    // Ordered list of paths in the queue (BTreeMap above sorts
    // lexicographically; we want insertion order to match the user's
    // selection / backend processing order).
    let (queue_order, set_queue_order) = signal::<Vec<String>>(Vec::new());
    let (busy, set_busy) = signal::<bool>(false);
    let (scanning, set_scanning) = signal::<bool>(false);
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

    Effect::new(move |_| {
        let cb = Closure::wrap(Box::new(move |e: JsValue| {
            let payload =
                js_sys::Reflect::get(&e, &JsValue::from_str("payload")).unwrap_or(JsValue::NULL);
            if let Ok(p) = serde_wasm_bindgen::from_value::<ProgressEvent>(payload) {
                set_progress.set(Some(format!(
                    "[{}/{}] {} — {}",
                    p.current,
                    p.total,
                    p.status,
                    basename(&p.path),
                )));
                // Update the visual queue for this path.
                let next_status = match p.status.as_str() {
                    "starting" => Some(QueueStatus::Running),
                    "imported" => Some(QueueStatus::Imported),
                    "skipped" => Some(QueueStatus::Skipped),
                    "failed" => Some(QueueStatus::Failed),
                    _ => None,
                };
                if let Some(ns) = next_status {
                    set_queue.update(|q| {
                        q.insert(p.path.clone(), ns);
                    });
                }
            }
        }) as Box<dyn FnMut(JsValue)>);
        spawn_local(async move {
            let _ = listen("dictionary-import-progress", &cb).await;
            cb.forget();
        });
    });

    let on_choose_folder = move |_| {
        set_scanning.set(true);
        set_banner.set(None);
        set_preview.set(Vec::new());
        set_selected.set(std::collections::HashSet::new());
        spawn_local(async move {
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
                    set_scanning.set(false);
                    return;
                }
            };
            let Some(path) = picked.as_string() else {
                set_scanning.set(false);
                return;
            };

            let args = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(&path));
            match invoke("scan_dictionary_folder", args.into()).await {
                Ok(v) => match serde_wasm_bindgen::from_value::<Vec<DictPreview>>(v) {
                    Ok(rows) => {
                        // Pre-select every Ready entry; leave the
                        // rest unchecked so accidental Imports don't
                        // try to re-process broken / unsupported zips.
                        let mut sel = std::collections::HashSet::new();
                        for r in &rows {
                            if r.status == "ready" {
                                sel.insert(r.path.clone());
                            }
                        }
                        set_selected.set(sel);
                        set_preview.set(rows);
                    }
                    Err(e) => set_banner.set(Some(format!("scan decode: {e}"))),
                },
                Err(e) => set_banner.set(Some(format!("scan: {}", stringify_err(e)))),
            }
            set_scanning.set(false);
        });
    };

    let on_import_selected = move |_| {
        // Preserve the user's selection order by walking the preview
        // rows instead of iterating the HashSet (which is unordered).
        let sel_set = selected.get();
        let chosen: Vec<String> = preview
            .get()
            .iter()
            .filter(|r| sel_set.contains(&r.path))
            .map(|r| r.path.clone())
            .collect();
        if chosen.is_empty() {
            set_banner.set(Some("Nothing selected.".into()));
            return;
        }
        set_busy.set(true);
        set_banner.set(None);
        set_progress.set(Some(format!("Queued {} dictionaries…", chosen.len())));
        // Seed the queue with every selected path in Queued state so
        // the UI shows the upcoming work before the first event lands.
        let mut q = std::collections::BTreeMap::new();
        for p in &chosen {
            q.insert(p.clone(), QueueStatus::Queued);
        }
        set_queue.set(q);
        set_queue_order.set(chosen.clone());
        spawn_local(async move {
            let args = js_sys::Object::new();
            let arr = js_sys::Array::new();
            for p in &chosen {
                arr.push(&JsValue::from_str(p));
            }
            let _ = js_sys::Reflect::set(&args, &JsValue::from_str("paths"), &arr);
            match invoke("import_dictionary_files", args.into()).await {
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
                    // Drop the scan preview + selection now that the
                    // installed list reflects the new state; keep the
                    // final queue table visible so the user can see
                    // per-zip results.
                    set_preview.set(Vec::new());
                    set_selected.set(std::collections::HashSet::new());
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

    let clear_queue = move |_| {
        set_queue.set(std::collections::BTreeMap::new());
        set_queue_order.set(Vec::new());
    };

    let toggle_all = move |checked: bool| {
        if checked {
            let mut sel = std::collections::HashSet::new();
            for r in preview.get() {
                if r.status == "ready" {
                    sel.insert(r.path);
                }
            }
            set_selected.set(sel);
        } else {
            set_selected.set(std::collections::HashSet::new());
        }
    };

    view! {
        <section class="dictionaries">
            <details open>
                <summary><h2 style="display:inline">"Dictionaries"</h2></summary>
                <div class="row">
                    <button
                        type="button"
                        on:click=on_choose_folder
                        prop:disabled=move || busy.get() || scanning.get()
                    >
                        {move || if scanning.get() { "Scanning…" } else { "Choose folder…" }}
                    </button>
                    {move || progress.get().map(|p| view! { <span class="muted">{p}</span> })}
                </div>
                {move || banner.get().map(|b| view! { <div class="banner">{b}</div> })}

                {move || {
                    let order = queue_order.get();
                    if order.is_empty() {
                        return view! { <span></span> }.into_any();
                    }
                    let map = queue.get();
                    // Index name lookups against the most recent
                    // preview scan so we can show dictionary titles in
                    // the queue rather than just file paths.
                    let names: std::collections::HashMap<String, String> = preview
                        .get()
                        .into_iter()
                        .filter_map(|r| r.name.clone().map(|n| (r.path, n)))
                        .collect();
                    let total = order.len();
                    let mut queued = 0usize;
                    let mut running = 0usize;
                    let mut imported = 0usize;
                    let mut skipped = 0usize;
                    let mut failed = 0usize;
                    for path in &order {
                        match map.get(path).cloned().unwrap_or(QueueStatus::Queued) {
                            QueueStatus::Queued => queued += 1,
                            QueueStatus::Running => running += 1,
                            QueueStatus::Imported => imported += 1,
                            QueueStatus::Skipped => skipped += 1,
                            QueueStatus::Failed => failed += 1,
                        }
                    }
                    let done = imported + skipped + failed;
                    view! {
                        <div class="dict-queue">
                            <div class="row">
                                <strong>"Import queue"</strong>
                                <span class="muted">
                                    {format!(
                                        "{done} / {total} done — \
                                         {imported} imported, {skipped} skipped, {failed} failed, \
                                         {running} running, {queued} queued"
                                    )}
                                </span>
                                {(!busy.get()).then(|| view! {
                                    <button type="button" on:click=clear_queue>"Clear"</button>
                                })}
                            </div>
                            <table class="dict-table dict-queue-table">
                                <thead>
                                    <tr>
                                        <th>"#"</th>
                                        <th>"Name"</th>
                                        <th>"Status"</th>
                                        <th>"File"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {order.iter().enumerate().map(|(i, path)| {
                                        let status = map.get(path).cloned()
                                            .unwrap_or(QueueStatus::Queued);
                                        let cls = status.css_class();
                                        let label = status.label();
                                        let display_name = names.get(path).cloned()
                                            .unwrap_or_else(|| basename(path));
                                        let file = basename(path);
                                        view! {
                                            <tr class=format!("queue-row {cls}")>
                                                <td class="muted">{i + 1}</td>
                                                <td>{display_name}</td>
                                                <td class=format!("queue-status {cls}")>{label}</td>
                                                <td class="muted dict-path">{file}</td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                    }.into_any()
                }}

                {move || {
                    let rows = preview.get();
                    if rows.is_empty() {
                        view! { <span></span> }.into_any()
                    } else {
                        let total_ready = rows.iter().filter(|r| r.status == "ready").count();
                        view! {
                            <div class="dict-preview">
                                <div class="row">
                                    <button
                                        type="button"
                                        on:click=move |_| toggle_all(true)
                                    >
                                        "Select all ready"
                                    </button>
                                    <button
                                        type="button"
                                        on:click=move |_| toggle_all(false)
                                    >
                                        "Clear selection"
                                    </button>
                                    <button
                                        type="button"
                                        on:click=on_import_selected
                                        prop:disabled=move || busy.get() || selected.get().is_empty()
                                    >
                                        {move || {
                                            let n = selected.get().len();
                                            if busy.get() {
                                                "Importing…".to_string()
                                            } else {
                                                format!("Import selected ({n})")
                                            }
                                        }}
                                    </button>
                                    <span class="muted">
                                        {format!("{} zip(s) found, {total_ready} ready", rows.len())}
                                    </span>
                                </div>
                                <table class="dict-table">
                                    <thead>
                                        <tr>
                                            <th></th>
                                            <th>"Name"</th>
                                            <th>"Status"</th>
                                            <th>"Format"</th>
                                            <th>"Path"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {rows.into_iter().map(|r| {
                                            let path_for_check = r.path.clone();
                                            let path_for_label = r.path.clone();
                                            let ready = r.status == "ready";
                                            let name_or_basename = r.name
                                                .clone()
                                                .unwrap_or_else(|| basename(&r.path));
                                            let status_label = match r.status.as_str() {
                                                "ready" => "Ready",
                                                "already-imported" => "Already imported",
                                                "unsupported-format" => "Unsupported format",
                                                "broken" => "Broken",
                                                other => other,
                                            }.to_string();
                                            let fmt = r.format_version
                                                .map(|f| format!("v{f}"))
                                                .unwrap_or_default();
                                            let on_check = move |ev: leptos::ev::Event| {
                                                let checked = leptos::prelude::event_target_checked(&ev);
                                                set_selected.update(|s| {
                                                    if checked {
                                                        s.insert(path_for_check.clone());
                                                    } else {
                                                        s.remove(&path_for_check);
                                                    }
                                                });
                                            };
                                            let is_checked = {
                                                let p = r.path.clone();
                                                move || selected.get().contains(&p)
                                            };
                                            view! {
                                                <tr>
                                                    <td>
                                                        <input
                                                            type="checkbox"
                                                            prop:disabled=!ready
                                                            prop:checked=is_checked
                                                            on:change=on_check
                                                        />
                                                    </td>
                                                    <td>{name_or_basename}</td>
                                                    <td class=move || format!("dict-status dict-status-{}", r.status)>
                                                        {status_label}
                                                    </td>
                                                    <td class="muted">{fmt}</td>
                                                    <td class="muted dict-path">{basename(&path_for_label)}</td>
                                                </tr>
                                            }
                                        }).collect_view()}
                                    </tbody>
                                </table>
                            </div>
                        }.into_any()
                    }
                }}

                {move || {
                    let rows = dicts.get();
                    if rows.is_empty() {
                        view! {
                            <p class="muted">"No dictionaries imported yet. Click \"Choose folder…\" to pick a directory of Yomitan .zip files."</p>
                        }.into_any()
                    } else {
                        view! {
                            <h3>"Installed"</h3>
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
