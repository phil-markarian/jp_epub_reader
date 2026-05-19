use leptos::html;
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
    #[serde(default)]
    author_romaji: String,
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
    offset: usize,
    limit: usize,
    #[serde(rename = "onlyPublicDomain")]
    only_public_domain: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SearchPage {
    rows: Vec<AozoraWork>,
    total: usize,
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
#[serde(rename_all = "camelCase")]
struct OpenArgs<'a> {
    path: &'a str,
    work_id: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteArgs {
    work_id: u32,
    delete_files: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenReaderArgs {
    work_id: u32,
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
    let (total, set_total) = signal::<usize>(0);
    let (page, set_page) = signal::<usize>(0);
    let (library, set_library) = signal::<Vec<LibraryEntry>>(Vec::new());
    let (lib_page, set_lib_page) = signal::<usize>(0);
    let (query, set_query) = signal(String::new());
    let (only_public, set_only_public) = signal(true);
    let (busy, set_busy) = signal(false);
    let (banner, set_banner) = signal::<Option<String>>(None);

    const PAGE_SIZE: usize = 50;
    const LIBRARY_PAGE_SIZE: usize = 25;

    // Which section the floating pager should target. Updated on scroll.
    let (active_section, set_active_section) = signal::<&'static str>("results");

    let library_section: NodeRef<html::Details> = NodeRef::new();
    let results_section: NodeRef<html::Details> = NodeRef::new();
    let index_section: NodeRef<html::Details> = NodeRef::new();
    crate::collapsible::persist_collapse(library_section, "library");
    crate::collapsible::persist_collapse(results_section, "results");
    crate::collapsible::persist_collapse(index_section, "index");

    Effect::new(move |_| {
        // Wait until both sections have mounted.
        let (Some(lib), Some(res)) = (library_section.get(), results_section.get()) else {
            return;
        };
        let lib_el: web_sys::HtmlElement = (*lib).clone().into();
        let res_el: web_sys::HtmlElement = (*res).clone().into();
        let Some(window) = web_sys::window() else { return };

        let recompute = move || {
            let scroll_y = window.scroll_y().unwrap_or(0.0);
            let viewport_h = window
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            // Use viewport-center as the focus point.
            let focus = scroll_y + viewport_h * 0.4;
            let res_top = res_el.offset_top() as f64;
            let lib_top = lib_el.offset_top() as f64;
            let active = if focus >= res_top {
                "results"
            } else if focus >= lib_top {
                "library"
            } else {
                "library"
            };
            set_active_section.set(active);
        };
        recompute();

        let cb = Closure::wrap(Box::new(recompute) as Box<dyn FnMut()>);
        let _ = web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("scroll", cb.as_ref().unchecked_ref());
        let _ = web_sys::window()
            .unwrap()
            .add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref());
        cb.forget();
    });

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

    let fetch_page = move |p: usize| {
        let q = query.get_untracked();
        let only = only_public.get_untracked();
        spawn_local(async move {
            match invoke_with(
                "search_works",
                &SearchArgs {
                    query: &q,
                    offset: p * PAGE_SIZE,
                    limit: PAGE_SIZE,
                    only_public_domain: only,
                },
            )
            .await
            {
                Ok(v) => {
                    if let Ok(page) = serde_wasm_bindgen::from_value::<SearchPage>(v) {
                        set_results.set(page.rows);
                        set_total.set(page.total);
                    }
                }
                Err(e) => set_banner.set(Some(format!("search: {e}"))),
            }
        });
    };

    // Re-search with page reset (used when query / filter changes).
    let do_search = move || {
        set_page.set(0);
        fetch_page(0);
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
            </header>

            <SourcesPanel status=status set_banner=set_banner refresh_status=refresh_status />
            <ImporterStatusLine importer=importer />

            <crate::dictionaries::DictionariesPanel />
            <LibraryPanel
                library=library
                refresh_library=refresh_library
                page=lib_page
                set_page=set_lib_page
                page_size=LIBRARY_PAGE_SIZE
                node_ref=library_section
            />

            <section class="search">
                <details open node_ref=index_section>
                    <summary><h2 style="display:inline">"Index"</h2></summary>
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
                    // Status banner — moved inside Index since most
                    // messages it carries are refresh / search related.
                    {move || banner.get().map(|b| view! { <div class="banner">{b}</div> })}
                </details>
            </section>

            <ResultsList
                results=results
                refresh_library=refresh_library
                library=library
                total=total
                page=page
                set_page=set_page
                fetch_page=fetch_page
                page_size=PAGE_SIZE
                node_ref=results_section
            />
            <SmartFloatingPager
                active_section=active_section
                results_total=total
                results_page=page
                set_results_page=set_page
                fetch_results_page=fetch_page
                results_page_size=PAGE_SIZE
                library=library
                lib_page=lib_page
                set_lib_page=set_lib_page
                lib_page_size=LIBRARY_PAGE_SIZE
                results_section=results_section
                library_section=library_section
            />
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

    let source_section: NodeRef<html::Details> = NodeRef::new();
    crate::collapsible::persist_collapse(source_section, "source");
    view! {
        <section class="sources">
            <details open node_ref=source_section>
                <summary><h2 style="display:inline">"Source"</h2></summary>
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
            </details>
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
    total: ReadSignal<usize>,
    page: ReadSignal<usize>,
    set_page: WriteSignal<usize>,
    fetch_page: impl Fn(usize) + Copy + 'static + Send + Sync,
    page_size: usize,
    node_ref: NodeRef<html::Details>,
) -> impl IntoView {
    view! {
        <details class="results" open=true node_ref=node_ref>
            <summary>
                <h2>"Results"</h2>
                <span class="muted summary-count">
                    {move || {
                        let t = total.get();
                        if t == 0 { String::new() } else { format!("({t})") }
                    }}
                </span>
            </summary>
            <PaginationBar
                total=total
                page=page
                set_page=set_page
                fetch_page=fetch_page
                page_size=page_size
            />
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
            <PaginationBar
                total=total
                page=page
                set_page=set_page
                fetch_page=fetch_page
                page_size=page_size
            />
        </details>
    }
}

#[component]
fn PaginationBar(
    total: ReadSignal<usize>,
    page: ReadSignal<usize>,
    set_page: WriteSignal<usize>,
    fetch_page: impl Fn(usize) + Copy + 'static + Send + Sync,
    page_size: usize,
) -> impl IntoView {
    let last_page = move || {
        let t = total.get();
        if t == 0 { 0 } else { (t - 1) / page_size }
    };
    let on_prev = move |_| {
        let p = page.get_untracked();
        if p > 0 { set_page.set(p - 1); fetch_page(p - 1); }
    };
    let on_next = move |_| {
        let p = page.get_untracked();
        if p < last_page() { set_page.set(p + 1); fetch_page(p + 1); }
    };

    view! {
        {move || {
            let t = total.get();
            if t == 0 || t <= page_size {
                return view! { <span></span> }.into_any();
            }
            let p = page.get();
            let lp = last_page();
            let from = p * page_size + 1;
            let to = ((p + 1) * page_size).min(t);
            view! {
                <div class="pagination">
                    <button type="button" on:click=on_prev prop:disabled=move || page.get() == 0>"‹ Prev"</button>
                    <span class="muted">
                        {format!("{from}-{to} of {t} · page {} / {}", p + 1, lp + 1)}
                    </span>
                    <button type="button" on:click=on_next prop:disabled=move || (page.get() >= last_page())>"Next ›"</button>
                </div>
            }.into_any()
        }}
    }
}

#[component]
fn SmartFloatingPager(
    active_section: ReadSignal<&'static str>,
    // Results pager wiring
    results_total: ReadSignal<usize>,
    results_page: ReadSignal<usize>,
    set_results_page: WriteSignal<usize>,
    fetch_results_page: impl Fn(usize) + Copy + 'static + Send + Sync,
    results_page_size: usize,
    // Library pager wiring (client-side paginated)
    library: ReadSignal<Vec<LibraryEntry>>,
    lib_page: ReadSignal<usize>,
    set_lib_page: WriteSignal<usize>,
    lib_page_size: usize,
    // For scroll-to-top on label click
    results_section: NodeRef<html::Details>,
    library_section: NodeRef<html::Details>,
) -> impl IntoView {
    let on_prev = move |_| match active_section.get() {
        "library" => {
            let p = lib_page.get_untracked();
            if p > 0 { set_lib_page.set(p - 1); }
        }
        _ => {
            let p = results_page.get_untracked();
            if p > 0 {
                set_results_page.set(p - 1);
                fetch_results_page(p - 1);
            }
        }
    };
    let on_next = move |_| match active_section.get() {
        "library" => {
            let total = library.get_untracked().len();
            let last = if total == 0 { 0 } else { (total - 1) / lib_page_size };
            let p = lib_page.get_untracked();
            if p < last { set_lib_page.set(p + 1); }
        }
        _ => {
            let total = results_total.get_untracked();
            let last = if total == 0 { 0 } else { (total - 1) / results_page_size };
            let p = results_page.get_untracked();
            if p < last {
                set_results_page.set(p + 1);
                fetch_results_page(p + 1);
            }
        }
    };

    let on_label_click = move |_| {
        let target = match active_section.get() {
            "library" => library_section.get(),
            _ => results_section.get(),
        };
        if let Some(el) = target {
            let html_el: web_sys::HtmlElement = (*el).clone().into();
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_behavior(web_sys::ScrollBehavior::Smooth);
            opts.set_block(web_sys::ScrollLogicalPosition::Start);
            html_el.scroll_into_view_with_scroll_into_view_options(&opts);
        }
    };

    view! {
        {move || {
            let section = active_section.get();
            let (page_now, last, total) = if section == "library" {
                let t = library.get().len();
                let last = if t == 0 { 0 } else { (t - 1) / lib_page_size };
                (lib_page.get(), last, t)
            } else {
                let t = results_total.get();
                let last = if t == 0 { 0 } else { (t - 1) / results_page_size };
                (results_page.get(), last, t)
            };
            // Hide if there's only one page in the active section.
            let page_size = if section == "library" { lib_page_size } else { results_page_size };
            if total <= page_size {
                return view! { <div></div> }.into_any();
            }
            let class = format!("floating-pager pager-{section}");
            let label = if section == "library" { "Library" } else { "Results" };
            let prev_disabled = page_now == 0;
            let next_disabled = page_now >= last;
            view! {
                <div class=class>
                    <button
                        type="button"
                        class="float-prev"
                        on:click=on_prev
                        prop:disabled=prev_disabled
                        title=format!("Previous page ({label})")
                        aria-label=format!("Previous page in {label}")
                    >"‹"</button>
                    <button
                        type="button"
                        class="float-section-label"
                        on:click=on_label_click
                        title=format!("Jump to top of {label}")
                    >{label}</button>
                    <button
                        type="button"
                        class="float-next"
                        on:click=on_next
                        prop:disabled=next_disabled
                        title=format!("Next page ({label})")
                        aria-label=format!("Next page in {label}")
                    >"›"</button>
                </div>
            }.into_any()
        }}
    }
}

#[component]
fn LibraryPanel(
    library: ReadSignal<Vec<LibraryEntry>>,
    refresh_library: impl Fn() + Copy + 'static + Send + Sync,
    page: ReadSignal<usize>,
    set_page: WriteSignal<usize>,
    page_size: usize,
    node_ref: NodeRef<html::Details>,
) -> impl IntoView {
    let total = move || library.get().len();
    let last_page = move || {
        let t = total();
        if t == 0 { 0 } else { (t - 1) / page_size }
    };
    let on_prev = move |_| {
        let p = page.get_untracked();
        if p > 0 { set_page.set(p - 1); }
    };
    let on_next = move |_| {
        let p = page.get_untracked();
        if p < last_page() { set_page.set(p + 1); }
    };
    let bar = move || {
        let t = total();
        if t <= page_size {
            return view! { <span></span> }.into_any();
        }
        let p = page.get();
        let from = p * page_size + 1;
        let to = ((p + 1) * page_size).min(t);
        view! {
            <div class="pagination">
                <button type="button" on:click=on_prev prop:disabled=move || page.get() == 0>"‹ Prev"</button>
                <span class="muted">
                    {format!("{from}-{to} of {t} · page {} / {}", p + 1, last_page() + 1)}
                </span>
                <button type="button" on:click=on_next prop:disabled=move || (page.get() >= last_page())>"Next ›"</button>
            </div>
        }.into_any()
    };

    view! {
        <details class="library" open=true node_ref=node_ref>
            <summary>
                <h2>"Library"</h2>
                <span class="muted summary-count">
                    {move || format!("({})", total())}
                </span>
            </summary>
            {bar}
            {move || {
                let rows = library.get();
                if rows.is_empty() {
                    return view! { <p class="muted">"Nothing imported yet — search below and click Import on a work."</p> }.into_any();
                }
                let p = page.get();
                let slice: Vec<LibraryEntry> = rows
                    .into_iter()
                    .skip(p * page_size)
                    .take(page_size)
                    .collect();
                view! {
                    <ul>
                        {slice.into_iter().map(|e| view! {
                            <li>
                                <LibraryRow entry=e refresh_library=refresh_library />
                            </li>
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
            {bar}
        </details>
    }
}

#[component]
fn LibraryRow(
    entry: LibraryEntry,
    refresh_library: impl Fn() + Copy + 'static + Send + Sync,
) -> impl IntoView {
    let title = entry.title.clone();
    let author = entry.author.clone().unwrap_or_default();
    let work_id = entry.work_id;
    let path_for_open = entry.epub_path.clone();

    let on_open = move |_| {
        let p = path_for_open.clone();
        spawn_local(async move {
            if let Err(e) = invoke_with(
                "open_path",
                &OpenArgs { path: &p, work_id: Some(work_id) },
            )
            .await
            {
                web_sys::console::error_1(&format!("open: {e}").into());
            }
        });
    };

    let on_read = move |_| {
        spawn_local(async move {
            if let Err(e) =
                invoke_with("open_reader_window", &OpenReaderArgs { work_id }).await
            {
                web_sys::console::error_1(&format!("open_reader: {e}").into());
            }
        });
    };

    // Two-click delete: first click arms; second click within ~3s
    // performs the deletion. Avoids window.confirm (blocked in
    // WKWebView) while still being explicit.
    let (armed, set_armed) = signal::<bool>(false);

    let on_remove = move |_| {
        if !armed.get_untracked() {
            set_armed.set(true);
            // Auto-disarm after 3s.
            spawn_local(async move {
                let win = web_sys::window().unwrap();
                let promise = js_sys::Promise::new(&mut |resolve, _| {
                    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                        &resolve, 3000,
                    );
                });
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                set_armed.set(false);
            });
            return;
        }
        set_armed.set(false);
        spawn_local(async move {
            if let Err(e) = invoke_with(
                "delete_library_entry",
                &DeleteArgs { work_id, delete_files: true },
            )
            .await
            {
                web_sys::console::error_1(&format!("remove: {e}").into());
                return;
            }
            refresh_library();
        });
    };

    view! {
        <div class="row work">
            <div class="meta">
                <div class="title"><strong>{title}</strong></div>
                <div class="author">{author}</div>
                <div class="ids muted">"work " {work_id}</div>
            </div>
            <div class="actions library-row-actions">
                <button type="button" on:click=on_read class="primary">"Read"</button>
                <button type="button" on:click=on_open title="Open with system default (Apple Books)">"Open externally"</button>
                <button
                    type="button"
                    class="library-row-trash"
                    on:click=on_remove
                    aria-label=move || if armed.get() { "Confirm removal" } else { "Remove from library" }.to_string()
                    title=move || if armed.get() { "Click again to confirm" } else { "Remove from library" }.to_string()
                >
                    {move || if armed.get() {
                        view! { <span class="library-row-confirm">"Confirm?"</span> }.into_any()
                    } else {
                        view! { <crate::icons::TrashIcon /> }.into_any()
                    }}
                </button>
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
            if let Err(e) = invoke_with(
                "open_path",
                &OpenArgs { path: &p, work_id: Some(work_id) },
            )
            .await
            {
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
