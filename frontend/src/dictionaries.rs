//! Phase 5 subpiece 1 — "Dictionaries" section in the main window.
//!
//! Two-step import flow:
//!   1. User picks a folder → recursive scan reads each zip's
//!      `index.json` and builds a checklist of preview rows.
//!   2. User selects which dictionaries to import → frontend loops
//!      through the chosen subset, invoking the backend's
//!      `import_single_dictionary` command ONE ZIP AT A TIME. Each
//!      `await` yields the JS event loop so the queue table redraws
//!      between imports and the UI never appears frozen.
//!
//! The queue table uses per-row `ArcRwSignal<QueueStatus>` so
//! flipping one row's status doesn't re-render the other 146 rows.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

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
    #[serde(default)]
    #[allow(dead_code)]
    revision: Option<String>,
    format_version: Option<i32>,
    status: String,
    #[serde(default)]
    #[allow(dead_code)]
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueueStatus {
    Queued,
    Running,
    Imported,
    Skipped,
    Failed,
    Cancelled,
}

impl QueueStatus {
    fn css_class(&self) -> &'static str {
        match self {
            Self::Queued => "queue-queued",
            Self::Running => "queue-running",
            Self::Imported => "queue-imported",
            Self::Skipped => "queue-skipped",
            Self::Failed => "queue-failed",
            Self::Cancelled => "queue-cancelled",
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Importing…",
            Self::Imported => "Imported",
            Self::Skipped => "Skipped",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

/// One row of the import queue. The status is its own signal so the
/// table doesn't re-render every row when one row's status flips —
/// fine-grained reactivity is what keeps the UI responsive when the
/// queue contains 100+ entries.
#[derive(Clone)]
struct QueueRow {
    index: usize,
    path: String,
    name: String,
    status: ArcRwSignal<QueueStatus>,
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

#[component]
pub fn DictionariesPanel() -> impl IntoView {
    let (dicts, set_dicts) = signal::<Vec<Dictionary>>(Vec::new());
    let (preview, set_preview) = signal::<Vec<DictPreview>>(Vec::new());
    let (selected, set_selected) = signal::<std::collections::HashSet<String>>(
        std::collections::HashSet::new(),
    );
    let (queue_rows, set_queue_rows) = signal::<Vec<QueueRow>>(Vec::new());
    let (counts, set_counts) = signal::<(usize, usize, usize, usize, usize)>((0, 0, 0, 0, 0));
    // (imported, skipped, failed, cancelled, done) — derived from
    // row signal updates inside the import loop.
    let (busy, set_busy) = signal::<bool>(false);
    let (scanning, set_scanning) = signal::<bool>(false);
    let (banner, set_banner) = signal::<Option<String>>(None);
    // Cancel flag — set by the modal's Cancel button. The import
    // loop reads this at the top of each iteration via get_untracked
    // and bails (marking remaining rows Cancelled) when true.
    let (cancel_read, cancel_write) = signal::<bool>(false);

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
        let sel_set = selected.get();
        let chosen: Vec<DictPreview> = preview
            .get()
            .into_iter()
            .filter(|r| sel_set.contains(&r.path))
            .collect();
        if chosen.is_empty() {
            set_banner.set(Some("Nothing selected.".into()));
            return;
        }
        // Build a queue row per chosen entry, each with its own
        // status signal. The Vec itself never mutates after this
        // point — only individual row signals flip — so the table
        // re-renders are O(1) per zip.
        let rows: Vec<QueueRow> = chosen
            .iter()
            .enumerate()
            .map(|(i, p)| QueueRow {
                index: i + 1,
                path: p.path.clone(),
                name: p
                    .name
                    .clone()
                    .unwrap_or_else(|| basename(&p.path)),
                status: ArcRwSignal::new(QueueStatus::Queued),
            })
            .collect();
        let total = rows.len();
        set_queue_rows.set(rows.clone());
        set_counts.set((0, 0, 0, 0, 0));
        set_busy.set(true);
        set_banner.set(None);
        // Reset cancel flag for the new batch.
        cancel_write.set(false);

        spawn_local(async move {
            let mut imported = 0usize;
            let mut skipped = 0usize;
            let mut failed = 0usize;
            let mut cancelled = 0usize;
            let mut iter = rows.into_iter();
            while let Some(row) = iter.next() {
                // Check the cancel flag BEFORE marking running so the
                // currently-running row isn't a phantom "Running".
                if cancel_read.get_untracked() {
                    row.status.set(QueueStatus::Cancelled);
                    cancelled += 1;
                    set_counts.set((
                        imported,
                        skipped,
                        failed,
                        cancelled,
                        imported + skipped + failed + cancelled,
                    ));
                    continue;
                }
                row.status.set(QueueStatus::Running);
                // Let the browser paint the Running state before
                // we block on the IPC. Two rAFs flush the style
                // change in WKWebView.
                yield_to_browser().await;

                let args = js_sys::Object::new();
                let _ = js_sys::Reflect::set(
                    &args,
                    &JsValue::from_str("path"),
                    &JsValue::from_str(&row.path),
                );
                let res = invoke("import_single_dictionary", args.into()).await;
                let next = match res {
                    Ok(v) => {
                        let kind = js_sys::Reflect::get(&v, &JsValue::from_str("kind"))
                            .ok()
                            .and_then(|x| x.as_string())
                            .unwrap_or_default();
                        match kind.as_str() {
                            "imported" => {
                                imported += 1;
                                QueueStatus::Imported
                            }
                            "skipped" => {
                                skipped += 1;
                                QueueStatus::Skipped
                            }
                            _ => {
                                failed += 1;
                                QueueStatus::Failed
                            }
                        }
                    }
                    Err(_) => {
                        failed += 1;
                        QueueStatus::Failed
                    }
                };
                row.status.set(next);
                set_counts.set((
                    imported,
                    skipped,
                    failed,
                    cancelled,
                    imported + skipped + failed + cancelled,
                ));
            }
            // Drain any remaining rows (the loop sets each to
            // Cancelled, but `iter` may have unyielded items if
            // the cancel arrived between iterations).
            for row in iter {
                row.status.set(QueueStatus::Cancelled);
                cancelled += 1;
            }
            set_counts.set((
                imported,
                skipped,
                failed,
                cancelled,
                imported + skipped + failed + cancelled,
            ));
            let summary = if cancelled > 0 {
                format!(
                    "{total} queued: {imported} imported, {skipped} skipped, {failed} failed, {cancelled} cancelled.",
                )
            } else {
                format!(
                    "{total} processed: {imported} imported, {skipped} skipped, {failed} failed.",
                )
            };
            set_banner.set(Some(summary));
            set_busy.set(false);
            set_preview.set(Vec::new());
            set_selected.set(std::collections::HashSet::new());
            refresh();
        });
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

    let clear_queue = move |_| {
        set_queue_rows.set(Vec::new());
        set_counts.set((0, 0, 0, 0, 0));
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
                </div>
                {move || banner.get().map(|b| view! { <div class="banner">{b}</div> })}

                // Modal import queue. Shown whenever there are queue
                // rows; busy controls Cancel vs Close button. The
                // backdrop and `aria-modal` semantics block clicks
                // on the rest of the page during import.
                {move || {
                    let rows = queue_rows.get();
                    if rows.is_empty() {
                        return view! { <span></span> }.into_any();
                    }
                    let total = rows.len();
                    let (imp, skp, fld, can, done) = counts.get();
                    let pct = if total > 0 {
                        (done as f64 / total as f64 * 100.0) as i32
                    } else { 0 };
                    view! {
                        <div class="dict-modal-backdrop" role="dialog" aria-modal="true">
                            <div class="dict-modal">
                                <header class="dict-modal-header">
                                    <h3>"Importing dictionaries"</h3>
                                    <span class="muted">
                                        {format!("{done} / {total}")}
                                    </span>
                                </header>
                                <div class="dict-progress-bar">
                                    <div class="dict-progress-fill"
                                        style=format!("width: {pct}%")></div>
                                </div>
                                <div class="dict-modal-counts muted">
                                    {format!(
                                        "{imp} imported • {skp} skipped • {fld} failed{}",
                                        if can > 0 { format!(" • {can} cancelled") } else { String::new() }
                                    )}
                                </div>
                                <div class="dict-modal-list">
                                    <table class="dict-table dict-queue-table">
                                        <thead>
                                            <tr>
                                                <th>"#"</th>
                                                <th>"Name"</th>
                                                <th>"Status"</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {rows.into_iter().map(|row| {
                                                let idx = row.index;
                                                let name = row.name.clone();
                                                let status = row.status.clone();
                                                view! {
                                                    <tr class="queue-row">
                                                        <td class="muted">{idx}</td>
                                                        <td>{name}</td>
                                                        <td>
                                                            {
                                                                let s = status.clone();
                                                                move || {
                                                                    let st = s.get();
                                                                    view! {
                                                                        <span class=format!("queue-status {}", st.css_class())>
                                                                            {st.label()}
                                                                        </span>
                                                                    }
                                                                }
                                                            }
                                                        </td>
                                                    </tr>
                                                }
                                            }).collect_view()}
                                        </tbody>
                                    </table>
                                </div>
                                <footer class="dict-modal-footer">
                                    {move || if busy.get() {
                                        let on_cancel = move |_| cancel_write.set(true);
                                        view! {
                                            <button
                                                type="button"
                                                class="dict-cancel-btn"
                                                on:click=on_cancel
                                                prop:disabled=move || cancel_read.get()
                                            >
                                                {move || if cancel_read.get() {
                                                    "Cancelling…"
                                                } else {
                                                    "Cancel"
                                                }}
                                            </button>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <button
                                                type="button"
                                                on:click=clear_queue
                                            >
                                                "Close"
                                            </button>
                                        }.into_any()
                                    }}
                                </footer>
                            </div>
                        </div>
                    }.into_any()
                }}

                // Scan preview / checklist.
                {move || {
                    let rows = preview.get();
                    if rows.is_empty() {
                        return view! { <span></span> }.into_any();
                    }
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
                }}

                // Installed list.
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

/// Yield to the browser long enough for it to paint the latest signal
/// updates. Two animation frames + a microtask is the rough lower
/// bound that reliably flushes a CSS class change in WKWebView; we
/// also add a small setTimeout(0) so the Cancel button's click event
/// has a chance to fire and flip the cancel flag between zips.
async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let win = web_sys::window().expect("no window");
        let resolve_cb = Closure::once_into_js(move || {
            let win2 = web_sys::window().expect("no window");
            let resolve2 = Closure::once_into_js(move || {
                let win3 = web_sys::window().expect("no window");
                let resolve3 = Closure::once_into_js(move || {
                    let _ = resolve.call0(&JsValue::NULL);
                });
                let _ = win3.set_timeout_with_callback_and_timeout_and_arguments_0(
                    resolve3.as_ref().unchecked_ref(),
                    0,
                );
            });
            let _ = win2.request_animation_frame(resolve2.as_ref().unchecked_ref());
        });
        let _ = win.request_animation_frame(resolve_cb.as_ref().unchecked_ref());
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
