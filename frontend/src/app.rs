use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

async fn invoke_no_args(cmd: &str) -> JsValue {
    invoke(cmd, JsValue::from_str("{}").into()).await
}

async fn invoke_with<T: Serialize>(cmd: &str, args: &T) -> JsValue {
    let v = serde_wasm_bindgen::to_value(args).unwrap();
    invoke(cmd, v).await
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
#[serde(rename_all = "camelCase")]
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

#[component]
pub fn App() -> impl IntoView {
    let (status, set_status) = signal::<Option<AozoraSourceStatus>>(None);
    let (results, set_results) = signal::<Vec<AozoraWork>>(Vec::new());
    let (query, set_query) = signal(String::new());
    let (only_public, set_only_public) = signal(true);
    let (busy, set_busy) = signal(false);
    let (banner, set_banner) = signal::<Option<String>>(None);

    let refresh_status = move || {
        spawn_local(async move {
            let v = invoke_no_args("get_aozora_source_status").await;
            if let Ok(s) = serde_wasm_bindgen::from_value::<AozoraSourceStatus>(v) {
                set_status.set(Some(s));
            }
        });
    };

    refresh_status();

    let do_search = move || {
        let q = query.get_untracked();
        let only = only_public.get_untracked();
        spawn_local(async move {
            let v = invoke_with(
                "search_works",
                &SearchArgs {
                    query: &q,
                    limit: 100,
                    only_public_domain: only,
                },
            )
            .await;
            if let Ok(rows) = serde_wasm_bindgen::from_value::<Vec<AozoraWork>>(v) {
                set_results.set(rows);
            }
        });
    };

    let on_refresh_index = move |_| {
        set_busy.set(true);
        set_banner.set(Some("Refreshing index…".into()));
        spawn_local(async move {
            let v = invoke_no_args("refresh_index").await;
            match serde_wasm_bindgen::from_value::<usize>(v) {
                Ok(n) => {
                    set_banner.set(Some(format!("Loaded {n} works.")));
                    refresh_status();
                    do_search();
                }
                Err(e) => set_banner.set(Some(format!("Refresh failed: {e:?}"))),
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

            <ResultsList results=results />
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
            let v = invoke_with("set_aozora_repo_path", &SetRepoArgs { path: &p }).await;
            if v.is_null() {
                set_banner.set(Some(format!("Local repo set: {p}")));
                refresh_status();
            } else if let Some(err) = v.as_string() {
                set_banner.set(Some(format!("Failed: {err}")));
            }
        });
    };

    let on_clear = move |_| {
        spawn_local(async move {
            let _ = invoke_no_args("clear_aozora_repo_path").await;
            set_banner.set(Some("Local repo cleared; using remote.".into()));
            refresh_status();
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
fn ResultsList(results: ReadSignal<Vec<AozoraWork>>) -> impl IntoView {
    view! {
        <section class="results">
            <h2>"Results"</h2>
            {move || {
                let rows = results.get();
                if rows.is_empty() {
                    view! { <p class="muted">"No results yet — refresh the index, then search."</p> }.into_any()
                } else {
                    view! {
                        <ul>
                            {rows.into_iter().map(|w| view! {
                                <li>
                                    <ResultRow work=w />
                                </li>
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
        </section>
    }
}

#[component]
fn ResultRow(work: AozoraWork) -> impl IntoView {
    let title = work.title.clone();
    let yomi = work.title_yomi.clone();
    let author = work.author.clone();
    let author_yomi = work.author_yomi.clone();
    let copyright = work.copyright_active;
    let stem = work.stem.clone();
    let author_id = work.author_id;
    let work_id = work.work_id;

    let on_resolve = move |_| {
        let Some(stem) = stem.clone() else { return };
        spawn_local(async move {
            let _v = invoke_with(
                "resolve_work",
                &ResolveArgs {
                    author_id,
                    stem: &stem,
                },
            )
            .await;
            // Result handling will be expanded in Phase 2 when this kicks off
            // an actual import. For now resolve just warms the on-disk cache.
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
            <button type="button" on:click=on_resolve>"Resolve"</button>
        </div>
    }
}
