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

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], catch)]
    async fn listen(
        event: &str,
        handler: &Closure<dyn FnMut(JsValue)>,
    ) -> Result<JsValue, JsValue>;
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportProgressEvent {
    path: String,
    current: u64,
    total: u64,
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
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    attribution: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    user_notes: Option<String>,
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
    /// When a row fails, the backend error message stored here is
    /// surfaced as a tooltip on hover and logged to the JS console
    /// so the user can diagnose what went wrong.
    error: ArcRwSignal<Option<String>>,
    /// Live (current, total) byte counts during the row's import;
    /// updated from the `dict-row-progress` Tauri event. Used to
    /// draw a progress bar inside the row's status cell.
    progress: ArcRwSignal<Option<(u64, u64)>>,
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

fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let n = n as f64;
    if n >= GB {
        format!("{:.2} GB", n / GB)
    } else if n >= MB {
        format!("{:.1} MB", n / MB)
    } else if n >= KB {
        format!("{:.0} KB", n / KB)
    } else {
        format!("{n:.0} B")
    }
}

#[component]
pub fn DictionariesPanel() -> impl IntoView {
    let (dicts, set_dicts) = signal::<Vec<Dictionary>>(Vec::new());
    // Which installed-dict rows have their Details panel expanded.
    // Keyed by dictionary id so the open state survives a refresh()
    // (since the same ids come back; only term_count etc. may
    // change).
    let (details_open, set_details_open) =
        signal::<std::collections::HashSet<i64>>(std::collections::HashSet::new());
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
    // Folder currently being scanned / last imported from. Surfaced
    // next to the "Choose folder…" button so the user can tell
    // which directory the checklist below corresponds to.
    let (current_folder, set_current_folder) = signal::<Option<String>>(None);
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

    // Run a scan against a given folder path, populating the preview
    // list. Extracted from on_choose_folder so we can call it both
    // from the dialog handler and from the mount-time auto-rescan.
    let run_scan = move |path: String| {
        set_scanning.set(true);
        set_banner.set(None);
        set_preview.set(Vec::new());
        set_selected.set(std::collections::HashSet::new());
        set_current_folder.set(Some(path.clone()));
        spawn_local(async move {
            let args = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &args,
                &JsValue::from_str("path"),
                &JsValue::from_str(&path),
            );
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

    refresh();

    // Subscribe to `dict-row-progress` events from the backend. We
    // do this once per panel mount; the closure looks up the
    // currently-queued row by its path string and pokes its
    // progress signal. Tauri's `listen` returns an unsubscribe
    // function that we deliberately leak — the panel never unmounts
    // in the current app.
    {
        let cb = Closure::wrap(Box::new(move |event: JsValue| {
            let payload =
                js_sys::Reflect::get(&event, &JsValue::from_str("payload"))
                    .unwrap_or(JsValue::NULL);
            let Ok(p) =
                serde_wasm_bindgen::from_value::<ImportProgressEvent>(payload)
            else {
                return;
            };
            for row in queue_rows.get_untracked() {
                if row.path == p.path {
                    row.progress.set(Some((p.current, p.total)));
                    break;
                }
            }
        }) as Box<dyn FnMut(JsValue)>);
        spawn_local(async move {
            let _ = listen("dict-row-progress", &cb).await;
            cb.forget();
        });
    }

    // On mount: if the user previously picked a folder, rescan it so
    // the panel comes up showing the same checklist (with up-to-date
    // "already imported" badges) instead of requiring a fresh
    // folder-pick every session.
    spawn_local(async move {
        if let Ok(v) = invoke("get_dict_last_folder", JsValue::from_str("{}")).await {
            if let Some(path) = v.as_string() {
                if !path.is_empty() {
                    run_scan(path);
                }
            }
        }
    });

    let on_choose_folder = move |_| {
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
                    return;
                }
            };
            let Some(path) = picked.as_string() else {
                return; // User cancelled.
            };
            run_scan(path);
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
                error: ArcRwSignal::new(None),
                progress: ArcRwSignal::new(None),
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
                                let err_msg = js_sys::Reflect::get(&v, &JsValue::from_str("error"))
                                    .ok()
                                    .and_then(|x| x.as_string())
                                    .unwrap_or_else(|| "unknown error".into());
                                web_sys::console::warn_1(
                                    &format!("[dict import] {} failed: {}", row.name, err_msg)
                                        .into(),
                                );
                                row.error.set(Some(err_msg));
                                failed += 1;
                                QueueStatus::Failed
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = stringify_err(e);
                        web_sys::console::warn_1(
                            &format!("[dict import] {} ipc failure: {}", row.name, err_msg)
                                .into(),
                        );
                        row.error.set(Some(err_msg));
                        failed += 1;
                        QueueStatus::Failed
                    }
                };
                row.status.set(next);
                // Drop the live progress now that the row is in a
                // terminal state — the cell switches back to a plain
                // status label.
                row.progress.set(None);
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
            set_selected.set(std::collections::HashSet::new());
            refresh();
            // Re-scan the saved folder (if any) so newly-imported
            // dicts flip to "Already imported" in the checklist
            // instead of disappearing. Falls through silently if no
            // folder was saved.
            if let Ok(v) = invoke("get_dict_last_folder", JsValue::from_str("{}")).await {
                if let Some(path) = v.as_string() {
                    if !path.is_empty() {
                        run_scan(path);
                    }
                }
            }
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

    // Prompt for a target folder, then move the given list of zip
    // paths into it via `move_dictionary_zips`. Used for both the
    // preview-side "Move broken/unsupported" button and the
    // queue-side "Move failed" button. After a successful move we
    // re-run the scan against the source folder so the rows for
    // the moved-away zips disappear from the preview.
    let move_zips_to_picked_dir = move |paths: Vec<String>| {
        if paths.is_empty() {
            set_banner.set(Some("Nothing to move (no problematic zips).".into()));
            return;
        }
        spawn_local(async move {
            let opts = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &opts,
                &JsValue::from_str("directory"),
                &JsValue::from_bool(true),
            );
            let picked = match open(opts.into()).await {
                Ok(v) => v,
                Err(e) => {
                    set_banner.set(Some(format!("dialog: {}", stringify_err(e))));
                    return;
                }
            };
            let Some(dir) = picked.as_string() else {
                return;
            };

            let args = js_sys::Object::new();
            let arr = js_sys::Array::new();
            for p in &paths {
                arr.push(&JsValue::from_str(p));
            }
            let _ = js_sys::Reflect::set(&args, &JsValue::from_str("paths"), &arr);
            let _ = js_sys::Reflect::set(
                &args,
                &JsValue::from_str("targetDir"),
                &JsValue::from_str(&dir),
            );
            match invoke("move_dictionary_zips", args.into()).await {
                Ok(v) => {
                    let arr = js_sys::Array::from(&v);
                    let total = arr.length() as usize;
                    let mut moved = 0usize;
                    let mut failed_msgs: Vec<String> = Vec::new();
                    for i in 0..arr.length() {
                        let entry = arr.get(i);
                        let kind = js_sys::Reflect::get(&entry, &JsValue::from_str("kind"))
                            .ok()
                            .and_then(|x| x.as_string())
                            .unwrap_or_default();
                        if kind == "moved" {
                            moved += 1;
                        } else {
                            let from = js_sys::Reflect::get(&entry, &JsValue::from_str("from"))
                                .ok()
                                .and_then(|x| x.as_string())
                                .unwrap_or_default();
                            let err = js_sys::Reflect::get(&entry, &JsValue::from_str("error"))
                                .ok()
                                .and_then(|x| x.as_string())
                                .unwrap_or_default();
                            let line = format!("{}: {}", basename(&from), err);
                            web_sys::console::warn_1(
                                &format!("[dict move] failed: {line}").into(),
                            );
                            failed_msgs.push(line);
                        }
                    }
                    let failed = failed_msgs.len();
                    let banner_msg = if failed == 0 {
                        format!("Moved {moved} dictionaries to {dir}")
                    } else {
                        // Surface the first failure inline so the
                        // user has something to act on without
                        // opening DevTools. Rest are in the console.
                        let first = failed_msgs.first().cloned().unwrap_or_default();
                        format!(
                            "Moved {moved} / {total} to {dir} — {failed} failed (first: {first})"
                        )
                    };
                    set_banner.set(Some(banner_msg));
                    // Re-scan the current dict folder so the moved
                    // rows drop out of the checklist.
                    if let Some(p) = current_folder.get_untracked() {
                        run_scan(p);
                    }
                }
                Err(e) => {
                    let msg = stringify_err(e);
                    web_sys::console::warn_1(
                        &format!("[dict move] backend returned err: {msg}").into(),
                    );
                    set_banner.set(Some(format!("move: {msg}")));
                }
            }
        });
    };

    let dict_section: NodeRef<leptos::html::Details> = NodeRef::new();
    crate::collapsible::persist_collapse(dict_section, "dictionaries");
    view! {
        <section class="dictionaries">
            <details open node_ref=dict_section>
                <summary><h2 style="display:inline">"Dictionaries"</h2></summary>
                <div class="row">
                    <button
                        type="button"
                        on:click=on_choose_folder
                        prop:disabled=move || busy.get() || scanning.get()
                    >
                        {move || if current_folder.get().is_some() {
                            if scanning.get() { "Scanning…" } else { "Change folder…" }
                        } else if scanning.get() { "Scanning…" } else { "Choose folder…" }}
                    </button>
                    {move || current_folder.get().map(|p| {
                        let title_attr = p.clone();
                        view! {
                            <span class="muted dict-current-folder" title=title_attr>
                                "Folder: " <code>{p}</code>
                            </span>
                        }
                    })}
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
                    let (imp, skp, fld, can, _done) = counts.get();
                    // Header counter shows only items the backend
                    // actually processed (imported / skipped / failed);
                    // cancelled rows don't count toward the X. After
                    // a Cancel mid-run this reads e.g. "12 / 66" with
                    // the rest reported in the counts line below.
                    let real_done = imp + skp + fld;
                    let pct = if total > 0 {
                        (real_done as f64 / total as f64 * 100.0) as i32
                    } else { 0 };
                    view! {
                        <div class="dict-modal-backdrop" role="dialog" aria-modal="true">
                            <div class="dict-modal">
                                <header class="dict-modal-header">
                                    <h3>"Importing dictionaries"</h3>
                                    <span class="muted">
                                        {format!("{real_done} / {total}")}
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
                                                let error = row.error.clone();
                                                let progress = row.progress.clone();
                                                view! {
                                                    <tr
                                                        class="queue-row"
                                                        title={
                                                            let err = error.clone();
                                                            move || err.get().unwrap_or_default()
                                                        }
                                                    >
                                                        <td class="muted">{idx}</td>
                                                        <td>{name}</td>
                                                        <td>
                                                            {
                                                                let s = status.clone();
                                                                let err = error.clone();
                                                                let prog = progress.clone();
                                                                move || {
                                                                    let st = s.get();
                                                                    let detail = if st == QueueStatus::Failed {
                                                                        err.get().map(|e| {
                                                                            let short = if e.len() > 80 {
                                                                                format!("{}…", &e[..80])
                                                                            } else { e };
                                                                            short
                                                                        })
                                                                    } else { None };
                                                                    let bar = if st == QueueStatus::Running {
                                                                        let (cur, tot) = prog.get().unwrap_or((0, 0));
                                                                        let pct = if tot > 0 {
                                                                            (cur as f64 / tot as f64 * 100.0).clamp(0.0, 100.0)
                                                                        } else { 0.0 };
                                                                        Some((pct, cur, tot))
                                                                    } else { None };
                                                                    // While running, replace the
                                                                    // "Importing…" pulse with just
                                                                    // the progress bar + numeric
                                                                    // readout. For other states
                                                                    // (Queued / Imported / Skipped
                                                                    // / Failed / Cancelled) keep
                                                                    // the text badge.
                                                                    let show_text = bar.is_none();
                                                                    view! {
                                                                        {show_text.then(|| view! {
                                                                            <span class=format!("queue-status {}", st.css_class())>
                                                                                {st.label()}
                                                                            </span>
                                                                        })}
                                                                        {bar.map(|(pct, cur, tot)| view! {
                                                                            <div class="queue-row-bar">
                                                                                <div
                                                                                    class="queue-row-bar-fill"
                                                                                    style=format!("width: {pct:.1}%")
                                                                                ></div>
                                                                            </div>
                                                                            <div class="queue-row-bar-label muted">
                                                                                {format!(
                                                                                    "{pct:.0}% — {}/{}",
                                                                                    format_bytes(cur),
                                                                                    format_bytes(tot),
                                                                                )}
                                                                            </div>
                                                                        })}
                                                                        {detail.map(|d| view! {
                                                                            <div class="queue-error muted">{d}</div>
                                                                        })}
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
                                        let on_cancel = move |_| {
                                            // 1) Stop the JS-side loop
                                            //    from queueing more zips.
                                            cancel_write.set(true);
                                            // 2) Tell the backend to
                                            //    abort the in-flight
                                            //    import; the
                                            //    transaction is
                                            //    dropped without
                                            //    commit so already-
                                            //    written rows are
                                            //    rolled back.
                                            spawn_local(async move {
                                                let _ = invoke(
                                                    "cancel_dictionary_import",
                                                    JsValue::from_str("{}"),
                                                ).await;
                                            });
                                        };
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
                                        // Gather any rows whose final
                                        // status is Failed so the user
                                        // can move them aside in one
                                        // click.
                                        let failed_paths: Vec<String> = queue_rows
                                            .get()
                                            .iter()
                                            .filter(|r| r.status.get_untracked() == QueueStatus::Failed)
                                            .map(|r| r.path.clone())
                                            .collect();
                                        let failed_count = failed_paths.len();
                                        view! {
                                            {(failed_count > 0).then(|| {
                                                let failed_paths = failed_paths.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        on:click=move |_| move_zips_to_picked_dir(failed_paths.clone())
                                                        title="Move zips that failed to import out to a separate folder"
                                                    >
                                                        {format!("Move failed ({failed_count})…")}
                                                    </button>
                                                }
                                            })}
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
                    let total = rows.len();
                    // Partition the scan into two buckets: things
                    // that still need importing (ready / unsupported /
                    // broken) vs. things already in the library. The
                    // already-imported bucket goes in a collapsed
                    // <details> so it's out of the way most of the time.
                    let ready_rows: Vec<DictPreview> = rows
                        .iter()
                        .filter(|r| r.status != "already-imported")
                        .cloned()
                        .collect();
                    let already_rows: Vec<DictPreview> = rows
                        .iter()
                        .filter(|r| r.status == "already-imported")
                        .cloned()
                        .collect();
                    let total_ready = ready_rows.iter().filter(|r| r.status == "ready").count();
                    let bad_paths: Vec<String> = rows
                        .iter()
                        .filter(|r| r.status == "broken" || r.status == "unsupported-format")
                        .map(|r| r.path.clone())
                        .collect();
                    let bad_count = bad_paths.len();
                    let already_count = already_rows.len();
                    view! {
                        <div class="dict-preview">
                            <h3>{format!("Dictionary sources ({} found)", total)}</h3>
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
                                {(bad_count > 0).then(|| {
                                    let bad_paths = bad_paths.clone();
                                    view! {
                                        <button
                                            type="button"
                                            on:click=move |_| move_zips_to_picked_dir(bad_paths.clone())
                                            prop:disabled=move || busy.get()
                                            title="Move broken / unsupported zip files to a folder of your choice"
                                        >
                                            {format!("Move problematic ({bad_count})…")}
                                        </button>
                                    }
                                })}
                            </div>
                            <div class="dict-installed-scroll dict-sources-scroll">
                                <details open class="dict-sources-section">
                                    <summary>
                                        <strong>{format!(
                                            "To install ({} ready, {} other)",
                                            total_ready,
                                            ready_rows.len().saturating_sub(total_ready),
                                        )}</strong>
                                    </summary>
                                    {render_preview_table(
                                        ready_rows,
                                        selected,
                                        set_selected,
                                    )}
                                </details>
                                {(already_count > 0).then(|| view! {
                                    <details class="dict-sources-section">
                                        <summary>
                                            <strong>{format!(
                                                "Already imported ({already_count})"
                                            )}</strong>
                                        </summary>
                                        {render_preview_table(
                                            already_rows,
                                            selected,
                                            set_selected,
                                        )}
                                    </details>
                                })}
                            </div>
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
                        let count = rows.len();
                        let last_idx = count.saturating_sub(1);
                        let ordered_ids: Vec<i64> = rows.iter().map(|d| d.id).collect();
                        let on_delete_all = move |_| {
                            // Two-click confirm pattern: first click
                            // arms (banner becomes red "click again to
                            // confirm"), second click within 5s wipes.
                            let armed_at = window_now_ms();
                            let prev = window_get_number("__JP_DICT_DELETE_ALL_AT")
                                .unwrap_or(0.0);
                            if armed_at - prev < 5000.0 && prev != 0.0 {
                                // Confirmed — wipe.
                                window_set_number("__JP_DICT_DELETE_ALL_AT", 0.0);
                                spawn_local(async move {
                                    if let Err(e) =
                                        invoke("delete_all_dictionaries", JsValue::from_str("{}"))
                                            .await
                                    {
                                        web_sys::console::warn_1(
                                            &format!("delete all: {}", stringify_err(e)).into(),
                                        );
                                    }
                                    refresh();
                                });
                            } else {
                                window_set_number("__JP_DICT_DELETE_ALL_AT", armed_at);
                                set_banner.set(Some(
                                    "Click \"Delete all\" again within 5 seconds to wipe every imported dictionary."
                                        .into(),
                                ));
                            }
                        };
                        view! {
                            <div class="row dict-installed-header">
                                <h3 style="margin: 0; flex: 1 1 auto;">{format!("Installed ({count})")}</h3>
                                <button
                                    type="button"
                                    class="dict-delete-all"
                                    on:click=on_delete_all
                                    title="Delete every imported dictionary (two-click confirm)"
                                >
                                    "🗑 Delete all"
                                </button>
                            </div>
                            <p class="muted dict-priority-hint">
                                "Drag the grip handle or use the arrows to reorder. "
                                "Higher in the list = higher priority in the lookup popup. "
                                "Toggle the checkbox to disable without deleting. "
                                "Click a row to view details & notes."
                            </p>
                            <div class="dict-installed-scroll">
                            <table class="dict-table dict-installed-table">
                                <thead>
                                    <tr>
                                        <th class="th-order"></th>
                                        <th class="th-on">"On"</th>
                                        <th>"Name"</th>
                                        <th>"Terms"</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {rows.into_iter().enumerate().map(|(i, d)| {
                                        let id = d.id;
                                        let name = d.name.clone();
                                        let tc = d.term_count;
                                        let enabled = d.enabled;
                                        let refresh = refresh;
                                        let ids_for_up = ordered_ids.clone();
                                        let ids_for_down = ordered_ids.clone();
                                        let ids_for_drag = ordered_ids.clone();
                                        let on_delete = {
                                            let n = name.clone();
                                            move |_| {
                                                let n = n.clone();
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
                                            }
                                        };
                                        let on_reimport = {
                                            let n = name.clone();
                                            move |_| {
                                                let n = n.clone();
                                                spawn_local(async move {
                                                    let args = js_sys::Object::new();
                                                    let _ = js_sys::Reflect::set(
                                                        &args,
                                                        &JsValue::from_str("id"),
                                                        &JsValue::from_f64(id as f64),
                                                    );
                                                    match invoke("reimport_dictionary", args.into()).await {
                                                        Ok(_) => refresh(),
                                                        Err(e) => web_sys::console::warn_1(
                                                            &format!("reimport {n}: {}", stringify_err(e)).into(),
                                                        ),
                                                    }
                                                });
                                            }
                                        };
                                        let on_toggle = move |ev: leptos::ev::Event| {
                                            let checked = leptos::prelude::event_target_checked(&ev);
                                            spawn_local(async move {
                                                let args = js_sys::Object::new();
                                                let _ = js_sys::Reflect::set(
                                                    &args,
                                                    &JsValue::from_str("id"),
                                                    &JsValue::from_f64(id as f64),
                                                );
                                                let _ = js_sys::Reflect::set(
                                                    &args,
                                                    &JsValue::from_str("enabled"),
                                                    &JsValue::from_bool(checked),
                                                );
                                                if let Err(e) =
                                                    invoke("set_dictionary_enabled", args.into()).await
                                                {
                                                    web_sys::console::warn_1(
                                                        &format!("set enabled: {}", stringify_err(e)).into(),
                                                    );
                                                }
                                                refresh();
                                            });
                                        };
                                        let on_move_up = move |_| {
                                            if i == 0 { return; }
                                            // Optimistic: swap the
                                            // local dicts signal
                                            // immediately so the row
                                            // visibly moves without
                                            // waiting for the IPC
                                            // roundtrip. send_reorder
                                            // fires in the background;
                                            // on failure we re-fetch
                                            // to restore truth.
                                            set_dicts.update(|d| {
                                                if i < d.len() { d.swap(i, i - 1); }
                                            });
                                            let mut next = ids_for_up.clone();
                                            next.swap(i, i - 1);
                                            spawn_local(async move {
                                                send_reorder(next).await;
                                            });
                                        };
                                        let on_move_down = move |_| {
                                            if i >= last_idx { return; }
                                            set_dicts.update(|d| {
                                                if i + 1 < d.len() { d.swap(i, i + 1); }
                                            });
                                            let mut next = ids_for_down.clone();
                                            next.swap(i, i + 1);
                                            spawn_local(async move {
                                                send_reorder(next).await;
                                            });
                                        };
                                        let up_disabled = i == 0;
                                        let down_disabled = i >= last_idx;

                                        // HTML5 drag-and-drop. The
                                        // source row index lives in
                                        // dataTransfer as a plain
                                        // string; the drop target
                                        // splices the source out of
                                        // its current position and
                                        // inserts it at the target's
                                        // index, then ships the new
                                        // order.
                                        let on_drag_start = move |ev: leptos::ev::DragEvent| {
                                            if let Some(dt) = ev.data_transfer() {
                                                let _ = dt.set_data("text/plain", &i.to_string());
                                                dt.set_effect_allowed("move");
                                            }
                                        };
                                        let on_drag_over = move |ev: leptos::ev::DragEvent| {
                                            // Default prevent so this <tr> is a valid drop target.
                                            ev.prevent_default();
                                            if let Some(dt) = ev.data_transfer() {
                                                dt.set_drop_effect("move");
                                            }
                                        };
                                        let on_drop = move |ev: leptos::ev::DragEvent| {
                                            ev.prevent_default();
                                            let from = ev
                                                .data_transfer()
                                                .and_then(|dt| dt.get_data("text/plain").ok())
                                                .and_then(|s| s.parse::<usize>().ok());
                                            let Some(from) = from else { return };
                                            if from == i { return }
                                            // Optimistic local splice.
                                            set_dicts.update(|d| {
                                                if from < d.len() && i < d.len() {
                                                    let row = d.remove(from);
                                                    let insert_at = if from < i { i } else { i };
                                                    d.insert(insert_at.min(d.len()), row);
                                                }
                                            });
                                            let mut next = ids_for_drag.clone();
                                            let moving = next.remove(from);
                                            let insert_at = if from < i { i } else { i };
                                            next.insert(insert_at.min(next.len()), moving);
                                            spawn_local(async move {
                                                send_reorder(next).await;
                                            });
                                        };

                                        // Clicking anywhere on the
                                        // row toggles the details
                                        // panel — except inside the
                                        // grip, arrow, checkbox or
                                        // delete cells, which have
                                        // their own actions. Detect
                                        // by walking up from the
                                        // click target and looking
                                        // for an .ignore-row-click
                                        // ancestor.
                                        let on_row_click = move |ev: leptos::ev::MouseEvent| {
                                            let target = ev.target()
                                                .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
                                            if let Some(el) = target {
                                                if el.closest(".ignore-row-click")
                                                    .ok()
                                                    .flatten()
                                                    .is_some()
                                                {
                                                    return;
                                                }
                                            }
                                            set_details_open.update(|s| {
                                                if !s.insert(id) { s.remove(&id); }
                                            });
                                        };

                                        view! {
                                            <tr
                                                class=if enabled { "dict-installed-row dict-installed-row-clickable" } else { "dict-installed-row dict-installed-row-clickable dict-row-disabled" }
                                                draggable="true"
                                                on:dragstart=on_drag_start
                                                on:dragover=on_drag_over
                                                on:drop=on_drop
                                                on:click=on_row_click
                                            >
                                                <td class="dict-order-cell ignore-row-click">
                                                    <div class="dict-order-controls">
                                                        <span class="dict-grip" title="Drag to reorder" aria-hidden="true">"⋮⋮"</span>
                                                        <div class="dict-arrows">
                                                            <button
                                                                type="button"
                                                                class="dict-arrow"
                                                                title="Move up"
                                                                prop:disabled=up_disabled
                                                                on:click=on_move_up
                                                            >"▲"</button>
                                                            <button
                                                                type="button"
                                                                class="dict-arrow"
                                                                title="Move down"
                                                                prop:disabled=down_disabled
                                                                on:click=on_move_down
                                                            >"▼"</button>
                                                        </div>
                                                    </div>
                                                </td>
                                                <td class="ignore-row-click">
                                                    <input
                                                        type="checkbox"
                                                        prop:checked=enabled
                                                        on:change=on_toggle
                                                    />
                                                </td>
                                                <td class="dict-name-cell">
                                                    <span class="dict-row-chevron">
                                                        {move || if details_open.get().contains(&id) { "▾" } else { "▸" }}
                                                    </span>
                                                    {d.name.clone()}
                                                </td>
                                                <td>{tc}</td>
                                                <td class="ignore-row-click dict-row-actions">
                                                    <button
                                                        type="button"
                                                        class="dict-icon-btn"
                                                        on:click=on_reimport
                                                        title="Reimport from source zip"
                                                        aria-label="Reimport dictionary"
                                                    >"⟳"</button>
                                                    <button
                                                        type="button"
                                                        class="dict-icon-btn dict-icon-danger"
                                                        on:click=on_delete
                                                        title="Delete dictionary"
                                                        aria-label="Delete dictionary"
                                                    >"🗑"</button>
                                                </td>
                                            </tr>
                                            {move || details_open.get().contains(&id).then(|| {
                                                let row = dicts.get_untracked()
                                                    .iter()
                                                    .find(|x| x.id == id)
                                                    .cloned();
                                                let Some(row) = row else {
                                                    return view! { <tr></tr> }.into_any();
                                                };
                                                view! {
                                                    <DictDetailsRow
                                                        row=row
                                                        refresh=refresh
                                                    />
                                                }.into_any()
                                            })}
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                            </div>
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

#[component]
fn DictDetailsRow(
    row: Dictionary,
    refresh: impl Fn() + Copy + 'static + Send + Sync,
) -> impl IntoView {
    let id = row.id;
    let initial_notes = row.user_notes.clone().unwrap_or_default();
    let (notes, set_notes) = signal::<String>(initial_notes.clone());
    let (saving, set_saving) = signal::<bool>(false);
    let (saved_at, set_saved_at) = signal::<Option<&'static str>>(None);

    let on_input = move |ev: leptos::ev::Event| {
        set_notes.set(leptos::prelude::event_target_value(&ev));
        set_saved_at.set(None);
    };

    let on_save = move |_| {
        set_saving.set(true);
        let value = notes.get_untracked();
        spawn_local(async move {
            let args = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &args,
                &JsValue::from_str("id"),
                &JsValue::from_f64(id as f64),
            );
            if value.is_empty() {
                let _ = js_sys::Reflect::set(
                    &args,
                    &JsValue::from_str("notes"),
                    &JsValue::NULL,
                );
            } else {
                let _ = js_sys::Reflect::set(
                    &args,
                    &JsValue::from_str("notes"),
                    &JsValue::from_str(&value),
                );
            }
            match invoke("set_dictionary_notes", args.into()).await {
                Ok(_) => {
                    set_saved_at.set(Some("Saved"));
                    refresh();
                }
                Err(e) => {
                    web_sys::console::warn_1(
                        &format!("set_dictionary_notes: {}", stringify_err(e)).into(),
                    );
                    set_saved_at.set(Some("Save failed"));
                }
            }
            set_saving.set(false);
        });
    };

    let revision = row.revision.clone().unwrap_or_else(|| "—".into());
    let description = row
        .description
        .clone()
        .unwrap_or_else(|| "(no description in index.json)".into());
    let attribution = row.attribution.clone();
    let url = row.url.clone();

    view! {
        <tr class="dict-details-row">
            <td colspan="5">
                <div class="dict-details">
                    <dl class="dict-details-meta">
                        <dt>"Format"</dt>
                        <dd>{format!("v{}", row.format_version)}</dd>
                        <dt>"Revision"</dt>
                        <dd>{revision}</dd>
                        {attribution.map(|a| view! {
                            <dt>"Attribution"</dt>
                            <dd>{a}</dd>
                        })}
                        {url.map(|u| {
                            let href = u.clone();
                            view! {
                                <dt>"URL"</dt>
                                <dd>
                                    <a href=href target="_blank" rel="noopener">{u}</a>
                                </dd>
                            }
                        })}
                        <dt>"Description"</dt>
                        <dd class="dict-details-description">{description}</dd>
                    </dl>
                    <label class="dict-details-notes-label">
                        "Your notes"
                        <textarea
                            class="dict-details-notes"
                            rows="3"
                            prop:value=move || notes.get()
                            on:input=on_input
                            placeholder="e.g. \"use only for kokugo lookups\""
                        />
                    </label>
                    <div class="row">
                        <button
                            type="button"
                            on:click=on_save
                            prop:disabled=move || saving.get()
                        >
                            {move || if saving.get() { "Saving…" } else { "Save notes" }}
                        </button>
                        {move || saved_at.get().map(|s| view! {
                            <span class="muted dict-details-saved">{s}</span>
                        })}
                    </div>
                </div>
            </td>
        </tr>
    }
}

/// Render the preview rows as a table. Extracted so both the
/// "To install" and "Already imported" sections can use the same
/// markup with different row buckets. `selected` is the parent's
/// signal of paths-checked-for-import; rows whose status isn't
/// "ready" have their checkbox disabled.
fn render_preview_table(
    rows: Vec<DictPreview>,
    selected: ReadSignal<std::collections::HashSet<String>>,
    set_selected: WriteSignal<std::collections::HashSet<String>>,
) -> impl IntoView {
    view! {
        <table class="dict-table dict-installed-table">
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
                    let path_for_row = r.path.clone();
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
                    let toggle = move |path: String| {
                        set_selected.update(|s| {
                            if s.contains(&path) { s.remove(&path); }
                            else { s.insert(path); }
                        });
                    };
                    let toggle_for_row = toggle;
                    let toggle_for_check = toggle;
                    let on_row_click = move |ev: leptos::ev::MouseEvent| {
                        if !ready { return; }
                        let target = ev.target()
                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
                        if let Some(el) = target {
                            if el.tag_name().eq_ignore_ascii_case("input") {
                                return;
                            }
                        }
                        toggle_for_row(path_for_row.clone());
                    };
                    let on_check = move |_ev: leptos::ev::Event| {
                        toggle_for_check(path_for_check.clone());
                    };
                    let is_checked = {
                        let p = r.path.clone();
                        move || selected.get().contains(&p)
                    };
                    let row_class = if ready {
                        "dict-preview-row"
                    } else {
                        "dict-preview-row dict-preview-row-disabled"
                    };
                    view! {
                        <tr class=row_class on:click=on_row_click>
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
    }
}

fn window_now_ms() -> f64 {
    js_sys::Date::now()
}

fn window_get_number(key: &str) -> Option<f64> {
    let win = web_sys::window()?;
    js_sys::Reflect::get(&win, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_f64())
}

fn window_set_number(key: &str, value: f64) {
    if let Some(win) = web_sys::window() {
        let _ = js_sys::Reflect::set(
            &win,
            &JsValue::from_str(key),
            &JsValue::from_f64(value),
        );
    }
}

async fn send_reorder(ordered_ids: Vec<i64>) {
    let args = js_sys::Object::new();
    let arr = js_sys::Array::new();
    for id in &ordered_ids {
        arr.push(&JsValue::from_f64(*id as f64));
    }
    let _ = js_sys::Reflect::set(&args, &JsValue::from_str("orderedIds"), &arr);
    if let Err(e) = invoke("reorder_dictionaries", args.into()).await {
        web_sys::console::warn_1(&format!("reorder: {}", stringify_err(e)).into());
    }
}
