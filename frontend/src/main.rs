mod app;
mod collapsible;
mod dictionaries;
mod reader;

use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();

    let reader_id = parse_reader_id();

    mount_to_body(move || match reader_id {
        Some(id) => leptos::view! { <reader::ReaderApp work_id=id /> }.into_any(),
        None => leptos::view! { <app::App /> }.into_any(),
    });
}

/// Parse `?reader=<u32>` out of `window.location.search`.
fn parse_reader_id() -> Option<u32> {
    let search = web_sys::window()?.location().search().ok()?;
    let trimmed = search.trim_start_matches('?');
    for pair in trimmed.split('&') {
        let mut it = pair.splitn(2, '=');
        let key = it.next()?;
        let value = it.next()?;
        if key == "reader" {
            return value.parse().ok();
        }
    }
    None
}
