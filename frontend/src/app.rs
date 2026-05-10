use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

async fn invoke_no_args(cmd: &str) -> Result<JsValue, String> {
    invoke(cmd, js_sys::Object::new().into())
        .await
        .map_err(stringify_err)
}

async fn invoke_with<T: Serialize>(cmd: &str, args: &T) -> Result<JsValue, String> {
    let v = serde_wasm_bindgen::to_value(args).unwrap();
    invoke(cmd, v).await.map_err(stringify_err)
}

fn stringify_err(v: JsValue) -> String {
    v.as_string()
        .or_else(|| js_sys::JSON::stringify(&v).ok().map(|s| s.into()))
        .unwrap_or_else(|| "unknown error".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AozoraWork {
    work_id: u32,
    author_id: u32,
    title: String,
    title_yomi: String,
    author: String,
    author_yomi: String,
    copyright_active: bool,
    stem: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AozoraSourceStatus {
    mode: String,
    repo_path: Option<String>,
    works_loaded: usize,
}

#[derive(Serialize)]
struct SearchArgs<'a> {
    query: &'a str,
    limit: usize,
    #[serde(rename = "onlyPublicDomain")]
    only_public_domain: bool,
}

#[derive(Serialize)]
struct SetRepoArgs<'a> {
    path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolveArgs<'a> {
    author_id: u32,
    stem: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportArgs {
    work_id: u32,
}

#[derive(Serialize)]
struct OpenArgs<'a> {
    path: &'a str,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ImportResult {
    epub_path: String,
    source_id: String,
    title: String,
    author: Option<String>,
    raw_text_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LibraryEntry {
    work_id: u32,
    title: String,
    author: Option<String>,
    epub_path: String,
    imported_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ImporterStatus {
    java: Option<JavaSummary>,
    jdk21_bundled: bool,
    original_bundled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JavaSummary {
    version_string: String,
    major: u32,
    binary: String,
}

#[component]
pub fn App() -> impl IntoView {
    let (status, set_status) = signal::<Option<AozoraSourceStatus>>(None);
    let (importer, set_importer) = signal::<Option<ImporterStatus>>(None);
    let (results, set_results) = signal::<Vec<AozoraWork>>(Vec::new());
    let (library, set_library) = signal::<Vec<LibraryEntry>>(Vec::new());
    let (query, set_query) = signal(String::new());
    let (only_public, set_only_public) = signal(true);
    let (busy, set_busy) = signal(false);
    let (banner, set_banner) = signal::<Option<String>>(None);

    let refresh_library = move || {
        spawn_local(async move {
            if let Ok(v) = invoke_no_args("list_library").await {
                if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<LibraryEntry>>(v) {
                    set_library.set(rows);
                }
            }
        });
    };

    refresh_library();

    spawn_local(async move {
        if let Ok(v) = invoke_no_args("get_importer_status").await {
            if let Ok(s) = serde_wasm_bindgen::from_value::<ImporterStatus>(v) {
                set_importer.set(Some(s));
            }
        }
    });

    let refresh_status = move || {
        spawn_local(async move {
            match invoke_no_args("get_aozora_source_status").await {
                Ok(v) => {
                    if let Ok(s) = serde_wasm_bindgen::from_value::<AozoraSourceStatus>(v) {
                        set_status.set(Some(s));
                    } else {
                        set_banner.set(Some("status: failed to decode response".into()));
                    }
                }
                Err(e) => set_banner.set(Some(format!("status: {e}"))),
            }
        });
    };

    refresh_status();

    let do_search = move || {
        let q = query.get_untracked();
        let only = only_public.get_untracked();
        spawn_local(async move {
            match invoke_with(
                "search_works",
                &SearchArgs {
                    query: &q,
                    limit: 100,
                    only_public_domain: only,
                },
            )
            .await
            {
                Ok(v) => {
                    if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<AozoraWork>>(v) {
                        set_results.set(rows);
                    }
                }
                Err(e) => set_banner.set(Some(format!("search: {e}"))),
            }
        });
    };

    let on_refresh_index = move |_| {
        set_busy.set(true);
        set_banner.set(Some("Refreshing index…".into()));
        spawn_local(async move {
            match invoke_no_args("refresh_index").await {
                Ok(v) => match serde_wasm_bindgen::from_value::<usize>(v) {
                    Ok(n) => {
                        set_banner.set(Some(format!("Loaded {n} works.")));
                        refresh_status();
                        do_search();
                    }
                    Err(e) => set_banner.set(Some(format!("refresh: bad payload: {e}"))),
                },
                Err(e) => set_banner.set(Some(format!("refresh: {e}"))),
            }
            set_busy.set(false);
        });
    };

    let on_query_input = move |ev: leptos::ev::Event| {
        let v = event_target_value(&ev);
        set_query.set(v);
        do_search();
    };

    let on_only_public_change = move |ev: leptos::ev::Event| {
        let checked = event_target_checked(&ev);
        set_only_public.set(checked);
        do_search();
    };

    view! {
        <main class="container">
            <header>
                <h1>"JP EPUB Reader"</h1>
                <p class="subtitle">"Phase 1 — Aozora source"</p>
            </header>

            <SourcesPanel status=status set_banner=set_banner refresh_status=refresh_status />
            <ImporterStatusLine importer=importer />

            {move || banner.get().map(|b| view! { <div class="banner">{b}</div> })}

            <section class="search">
                <div class="row">
                    <input
                        type="text"
                        placeholder="Search title, author, reading…"
                        prop:value=move || query.get()
                        on:input=on_query_input
                    />
                    <button
                        type="button"
                        prop:disabled=move || busy.get()
                        on:click=on_refresh_index
                    >
                        {move || if busy.get() { "Refreshing…" } else { "Refresh index" }}
                    </button>
                </div>
                <label class="row">
                    <input
                        type="checkbox"
                        prop:checked=move || only_public.get()
                        on:change=on_only_public_change
                    />
                    "Public domain only"
                </label>
            </section>

            <LibraryPanel library=library refresh_library=refresh_library />
            <ResultsList results=results refresh_library=refresh_library library=library />
        </main>
    }
}

#[component]
fn SourcesPanel(
    status: ReadSignal<Option<AozoraSourceStatus>>,
    set_banner: WriteSignal<Option<String>>,
    refresh_status: impl Fn() + Copy + 'static + Send + Sync,
) -> impl IntoView {
    let (path_input, set_path_input) = signal(String::new());

    Effect::new(move |_| {
        if let Some(s) = status.get() {
            if let Some(p) = s.repo_path {
                set_path_input.set(p);
            }
        }
    });

    let on_set_path = move |_| {
        let p = path_input.get_untracked();
        spawn_local(async move {
            match invoke_with("set_aozora_repo_path", &SetRepoArgs { path: &p }).await {
                Ok(_) => {
                    set_banner.set(Some(format!("Local repo set: {p}")));
                    refresh_status();
                }
                Err(e) => set_banner.set(Some(format!("Failed: {e}"))),
            }
        });
    };

    let on_clear = move |_| {
        spawn_local(async move {
            match invoke_no_args("clear_aozora_repo_path").await {
                Ok(_) => {
                    set_banner.set(Some("Local repo cleared; using remote.".into()));
                    refresh_status();
                }
                Err(e) => set_banner.set(Some(format!("Clear failed: {e}"))),
            }
        });
    };

    view! {
        <section class="sources">
            <h2>"Source"</h2>
            {move || match status.get() {
                None => view! { <p>"Loading status…"</p> }.into_any(),
                Some(s) => {
                    let mode = s.mode.clone();
                    let repo = s.repo_path.clone();
                    let loaded = s.works_loaded;
                    view! {
                        <p>
                            "Mode: " <strong>{mode}</strong>
                            " · Works loaded: " <strong>{loaded}</strong>
                        </p>
                        {repo.map(|p| view! {
                            <p class="muted">"Local repo: " <code>{p}</code></p>
                        })}
                    }.into_any()
                }
            }}

            <div class="row">
                <input
                    type="text"
                    placeholder="/path/to/aozorabunko_text"
                    prop:value=move || path_input.get()
                    on:input=move |ev| set_path_input.set(event_target_value(&ev))
                />
                <button type="button" on:click=on_set_path>"Use local repo"</button>
                <button type="button" on:click=on_clear>"Use remote"</button>
            </div>
        </section>
    }
}

#[component]
fn ImporterStatusLine(importer: ReadSignal<Option<ImporterStatus>>) -> impl IntoView {
    view! {
        <p class="muted">
            {move || match importer.get() {
                None => "Importer: checking…".to_string(),
                Some(s) => {
                    let java = match s.java {
                        Some(j) => format!("java {}", j.major),
                        None => "java MISSING".to_string(),
                    };
                    let jars = match (s.jdk21_bundled, s.original_bundled) {
                        (true, true) => "jdk21 + original".to_string(),
                        (true, false) => "jdk21".to_string(),
                        (false, true) => "original".to_string(),
                        (false, false) => "no jars bundled".to_string(),
                    };
                    format!("Importer: {java} · jars: {jars}")
                }
            }}
        </p>
    }
}

#[component]
fn ResultsList(
    results: ReadSignal<Vec<AozoraWork>>,
    refresh_library: impl Fn() + Copy + 'static + Send + Sync,
    library: ReadSignal<Vec<LibraryEntry>>,
) -> impl IntoView {
    view! {
        <details class="results" open=true>
            <summary><h2>"Results"</h2></summary>
            {move || {
                let rows = results.get();
                if rows.is_empty() {
                    view! { <p class="muted">"No results yet — refresh the index, then search."</p> }.into_any()
                } else {
                    view! {
                        <ul>
                            {rows.into_iter().map(|w| view! {
                                <li>
                                    <ResultRow
                                        work=w
                                        refresh_library=refresh_library
                                        library=library
                                    />
                                </li>
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
        </details>
    }
}

#[component]
fn LibraryPanel(
    library: ReadSignal<Vec<LibraryEntry>>,
    refresh_library: impl Fn() + Copy + 'static + Send + Sync,
) -> impl IntoView {
    view! {
        <details class="library" open=true>
            <summary>
                <h2>"Library"</h2>
                <span class="muted summary-count">
                    {move || format!("({})", library.get().len())}
                </span>
            </summary>
            {move || {
                let rows = library.get();
                if rows.is_empty() {
                    view! { <p class="muted">"Nothing imported yet — search below and click Import on a work."</p> }.into_any()
                } else {
                    view! {
                        <ul>
                            {rows.into_iter().map(|e| view! {
                                <li>
                                    <LibraryRow entry=e refresh_library=refresh_library />
                                </li>
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
        </details>
    }
}

#[component]
fn LibraryRow(
    entry: LibraryEntry,
    refresh_library: impl Fn() + Copy + 'static + Send + Sync,
) -> impl IntoView {
    let _ = refresh_library;
    let title = entry.title.clone();
    let author = entry.author.clone().unwrap_or_default();
    let work_id = entry.work_id;
    let path_for_open = entry.epub_path.clone();
    let on_open = move |_| {
        let p = path_for_open.clone();
        spawn_local(async move {
            if let Err(e) = invoke_with("open_path", &OpenArgs { path: &p }).await {
                web_sys::console::error_1(&format!("open: {e}").into());
            }
        });
    };

    view! {
        <div class="row work">
            <div class="meta">
                <div class="title"><strong>{title}</strong></div>
                <div class="author">{author}</div>
                <div class="ids muted">"work " {work_id}</div>
            </div>
            <div class="actions">
                <button type="button" on:click=on_open>"Open"</button>
            </div>
        </div>
    }
}

#[component]
fn ResultRow(
    work: AozoraWork,
    refresh_library: impl Fn() + Copy + 'static + Send + Sync,
    library: ReadSignal<Vec<LibraryEntry>>,
) -> impl IntoView {
    let title = work.title.clone();
    let yomi = work.title_yomi.clone();
    let author = work.author.clone();
    let author_yomi = work.author_yomi.clone();
    let copyright = work.copyright_active;
    let stem = work.stem.clone();
    let author_id = work.author_id;
    let work_id = work.work_id;

    let (epub_path, set_epub_path) = signal::<Option<String>>(None);
    let (importing, set_importing) = signal(false);
    let (row_error, set_row_error) = signal::<Option<String>>(None);

    let stem_for_resolve = stem.clone();
    let on_resolve = move |_| {
        let Some(stem) = stem_for_resolve.clone() else { return };
        spawn_local(async move {
            let res = invoke_with(
                "resolve_work",
                &ResolveArgs {
                    author_id,
                    stem: &stem,
                },
            )
            .await;
            match res {
                Ok(v) => web_sys::console::log_2(&"resolve_work ok:".into(), &v),
                Err(e) => web_sys::console::error_1(&format!("resolve_work: {e}").into()),
            }
        });
    };

    let stem_present = stem.is_some();

    // Reactive: is this work already in the library?
    let in_library = move || library.get().iter().any(|e| e.work_id == work_id);

    // Reactive: epub_path of the library entry, if any. Lets us show
    // Open without waiting for a fresh import in this session.
    let library_epub = move || {
        library
            .get()
            .iter()
            .find(|e| e.work_id == work_id)
            .map(|e| e.epub_path.clone())
    };

    let on_import = move |_| {
        if !stem_present { return }
        set_importing.set(true);
        set_row_error.set(None);
        spawn_local(async move {
            match invoke_with("import_aozora_work", &ImportArgs { work_id }).await {
                Ok(v) => match serde_wasm_bindgen::from_value::<ImportResult>(v) {
                    Ok(r) => {
                        set_epub_path.set(Some(r.epub_path));
                        refresh_library();
                    }
                    Err(e) => set_row_error.set(Some(format!("bad payload: {e}"))),
                },
                Err(e) => set_row_error.set(Some(e)),
            }
            set_importing.set(false);
        });
    };

    let on_open = move |_| {
        let p = epub_path.get_untracked().or_else(library_epub);
        let Some(p) = p else { return };
        spawn_local(async move {
            if let Err(e) = invoke_with("open_path", &OpenArgs { path: &p }).await {
                set_row_error.set(Some(format!("open: {e}")));
            }
        });
    };

    view! {
        <div class="row work">
            <div class="meta">
                <div class="title">
                    <strong>{title}</strong>
                    <span class="yomi">" / " {yomi}</span>
                </div>
                <div class="author">
                    {author}
                    <span class="yomi">" (" {author_yomi} ")"</span>
                </div>
                <div class="ids muted">
                    "work " {work_id} " · author " {author_id}
                    {copyright.then(|| view! { <span class="badge">" © active"</span> })}
                </div>
            </div>
            <div class="actions">
                <button type="button" on:click=on_resolve>"Resolve"</button>
                {move || {
                    let just_imported = epub_path.get().is_some();
                    let stored = in_library();
                    let busy = importing.get();
                    if just_imported || stored {
                        let label = if busy { "Re-importing…" } else { "Re-import" };
                        view! {
                            <span class="imported-flash">"✓ EPUB"</span>
                            <button type="button" on:click=on_open>"Open"</button>
                            <button
                                type="button"
                                on:click=on_import
                                prop:disabled=move || importing.get()
                                title="Overwrite the existing EPUB"
                            >{label}</button>
                        }.into_any()
                    } else {
                        view! {
                            <button
                                type="button"
                                on:click=on_import
                                prop:disabled=move || importing.get()
                            >
                                {move || if importing.get() { "Importing…" } else { "Import" }}
                            </button>
                        }.into_any()
                    }
                }}
            </div>
        </div>
        {move || row_error.get().map(|e| view! { <div class="row-error">{e}</div> })}
    }
}
