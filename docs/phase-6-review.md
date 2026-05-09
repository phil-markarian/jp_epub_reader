# Phase 6 — Review Surfaces

**Goal:** menu bar app, daily notification, word-of-day window, weekly
HTML report. All driven by vocab DB queries.

**Time:** one weekend.

After 1-3 months of using Phases 0-5 daily, you have enough vocab data
to make these features useful. Don't build them sooner.

## Menu bar app

Tauri 2 supports tray icons via `TrayIconBuilder`. The icon sits in the
macOS menu bar and shows a dropdown with quick stats and recent words.

```rust
// src-tauri/src/tray.rs

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIconBuilder, MouseButton, MouseButtonState},
    AppHandle, Manager,
};

pub fn setup_tray(app: &AppHandle) -> Result<()> {
    let stats_item = MenuItem::with_id(app, "stats", "—", false, None::<&str>)?;
    let recent_item = MenuItem::with_id(app, "recent", "Recent words", true, None::<&str>)?;
    let review_item = MenuItem::with_id(app, "review", "Review tracked words", true, None::<&str>)?;
    let open_item = MenuItem::with_id(app, "open", "Open library", true, None::<&str>)?;
    let quit_item = PredefinedMenuItem::quit(app, Some("Quit"))?;

    let menu = Menu::with_items(app, &[
        &stats_item,
        &PredefinedMenuItem::separator(app)?,
        &recent_item,
        &review_item,
        &PredefinedMenuItem::separator(app)?,
        &open_item,
        &quit_item,
    ])?;

    let _tray = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => { app.get_webview_window("main").unwrap().show().unwrap(); }
            "review" => { open_review_window(app); }
            "recent" => { open_recent_window(app); }
            _ => {}
        })
        .build(app)?;

    // Refresh stats label periodically
    let app_handle = app.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            update_tray_stats(&app_handle).await;
        }
    });

    Ok(())
}

async fn update_tray_stats(app: &AppHandle) {
    let state = app.state::<AppState>();
    let stats = state.vocab.lock().unwrap().today_stats().unwrap_or_default();
    // Update menu item label: "12 today, 47 this week"
    // Tauri's MenuItem doesn't allow direct text update; rebuild submenu or
    // use a Submenu and rebuild items. Simplest: rebuild the whole menu.
}
```

The stats query for the menu bar:

```rust
pub fn today_stats(&self) -> Result<TodayStats> {
    let today_start = /* unix timestamp for start of local day */;
    let week_start = today_start - 6 * 86400;

    Ok(TodayStats {
        words_today: self.conn.query_row(
            "SELECT COUNT(DISTINCT vocab_id) FROM encounter
             WHERE occurred_at >= ?1",
            params![today_start], |r| r.get(0))?,
        words_this_week: self.conn.query_row(
            "SELECT COUNT(DISTINCT vocab_id) FROM encounter
             WHERE occurred_at >= ?1",
            params![week_start], |r| r.get(0))?,
        tracked_pending: self.conn.query_row(
            "SELECT COUNT(*) FROM vocab WHERE status = 'tracked'",
            [], |r| r.get(0))?,
    })
}
```

## Daily notification

Schedule a notification at a user-set time (default 8am local) showing
yesterday's stats and a prompt to review.

```rust
use tauri_plugin_notification::NotificationExt;

pub async fn schedule_daily_notification(app: AppHandle) {
    loop {
        let now = chrono::Local::now();
        let target = compute_next_notification_time(&now);
        let delay = (target - now).to_std().unwrap_or(Duration::ZERO);
        tokio::time::sleep(delay).await;

        let state = app.state::<AppState>();
        let stats = state.vocab.lock().unwrap().yesterday_stats().unwrap_or_default();

        if stats.words_seen > 0 {
            app.notification()
                .builder()
                .title(format!("Japanese: {} words yesterday", stats.words_seen))
                .body(format!(
                    "{} new, {} tracked. {} pending review.",
                    stats.new_words, stats.tracked_added, stats.tracked_pending
                ))
                .show()
                .ok();
        }
    }
}
```

The notification respects macOS Do Not Disturb settings — that's
handled by macOS, not us.

Settings expose: enable/disable, time of day, content options
(stats only / stats + word of day / minimal).

## Word-of-day floating window

Small always-on-top window that opens at a scheduled time, showing one
word from your `tracked` list with its sentence. Click to mark known,
mine to Anki, dismiss, or "next."

```rust
pub async fn show_word_of_day(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let word = state.vocab.lock().unwrap().pick_word_of_day()?;

    let label = format!("word-of-day-{}", word.id);
    if app.get_webview_window(&label).is_some() {
        return Ok(());  // already showing
    }

    let url = format!("/word-of-day?vocab_id={}", word.id);
    WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::App(url.into()),
    )
    .title("Word of the day")
    .inner_size(380.0, 280.0)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .build()?;
    Ok(())
}
```

The picker algorithm:

```rust
pub fn pick_word_of_day(&self) -> Result<VocabRow> {
    // Strategy: weight tracked words by how recently we've shown them
    // (least-recently shown wins) + a small random factor.
    //
    // Implementation: track shown_count + last_shown_at on a separate
    // wod_history table to avoid polluting vocab.
    //
    // Initial pick query:
    self.conn.query_row(
        "SELECT v.* FROM vocab v
         LEFT JOIN wod_history h ON h.vocab_id = v.id
         WHERE v.status IN ('tracked', 'learning')
         ORDER BY COALESCE(h.last_shown_at, 0) ASC, RANDOM()
         LIMIT 1",
        [], VocabRow::from_row,
    )
}
```

Word-of-day window UI: shows the word, reading, definition, the
sentence where you first encountered it, and 4 buttons (mark known /
mine to Anki / next / dismiss). Same `LookupPopup` component from Phase
5 with minor tweaks.

Capability `src-tauri/capabilities/word-of-day.json`:

```json
{
  "identifier": "word-of-day",
  "windows": ["word-of-day-*"],
  "permissions": [
    "core:default",
    {
      "identifier": "http:default",
      "allow": [{ "url": "http://127.0.0.1:8765/*" }]
    }
  ]
}
```

Read-only access to the vocab DB through a narrow set of commands:

```rust
#[tauri::command]
pub fn wod_get(vocab_id: i64) -> Result<WordOfDayPayload, String>;
#[tauri::command]
pub fn wod_set_status(vocab_id: i64, status: String) -> Result<(), String>;
#[tauri::command]
pub fn wod_dismiss(vocab_id: i64) -> Result<(), String>;
#[tauri::command]
pub fn wod_next() -> Result<WordOfDayPayload, String>;
```

## Weekly HTML report

Generated locally, opens in the user's default browser. Static HTML —
no server, no live data, no JS framework.

Triggered manually ("Generate weekly report" in main window) or
automatically every Sunday at 6pm.

```rust
// crates/jp-vocab/src/report.rs

pub fn weekly_report(db: &VocabDb) -> Result<String> {
    let week_start = /* 7 days ago */;
    let stats = db.weekly_stats(week_start)?;
    let html = render_report(&stats);
    Ok(html)
}

fn render_report(stats: &WeeklyStats) -> String {
    // Use askama or maud or just format!() for templating.
    // Keep it minimal and printable.
    format!(r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>Japanese — week of {date}</title>
<style>
body {{ font-family: -apple-system, sans-serif; max-width: 720px; margin: 2rem auto; padding: 1rem; }}
h1, h2 {{ color: #222; }}
.heatmap {{ display: grid; grid-template-columns: repeat(7, 1fr); gap: 2px; }}
.heatmap div {{ aspect-ratio: 1; background: #eee; }}
.heatmap div.l1 {{ background: #c6e48b; }}
.heatmap div.l2 {{ background: #7bc96f; }}
.heatmap div.l3 {{ background: #239a3b; }}
.heatmap div.l4 {{ background: #196127; }}
table {{ border-collapse: collapse; width: 100%; }}
td, th {{ padding: 0.4rem; border-bottom: 1px solid #eee; text-align: left; }}
.status-tracked {{ color: #d97706; }}
.status-learning {{ color: #2563eb; }}
.status-known {{ color: #15803d; }}
</style>
</head>
<body>
<h1>Week of {date}</h1>

<h2>At a glance</h2>
<ul>
  <li>{words_seen} unique words encountered</li>
  <li>{new_words} first-time encounters</li>
  <li>{newly_tracked} added to tracked</li>
  <li>{newly_mined} cards mined to Anki</li>
  <li>Total reading time across sources: {reading_time}</li>
</ul>

<h2>Activity</h2>
<div class="heatmap">{heatmap}</div>

<h2>Sources read</h2>
<table>
<tr><th>Source</th><th>Encounters</th><th>Lookup density</th></tr>
{sources}
</table>

<h2>Newly tracked words</h2>
<table>
<tr><th>Word</th><th>Reading</th><th>First sentence</th></tr>
{tracked}
</table>

<h2>Words that promoted to known</h2>
<table>
<tr><th>Word</th><th>Reading</th></tr>
{promoted}
</table>

</body>
</html>"#,
        date = format_date_range(stats.week_start),
        words_seen = stats.words_seen,
        new_words = stats.new_words,
        newly_tracked = stats.newly_tracked,
        newly_mined = stats.newly_mined,
        reading_time = stats.format_reading_time(),
        heatmap = render_heatmap(&stats.daily_counts),
        sources = render_sources(&stats.sources),
        tracked = render_tracked(&stats.newly_tracked_words),
        promoted = render_promoted(&stats.promoted_to_known),
    )
}
```

Save to a local file, open with `open` crate:

```rust
#[tauri::command]
pub fn generate_weekly_report(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let db = state.vocab.lock().unwrap();
    let html = vocab::report::weekly_report(&db).map_err(|e| e.to_string())?;
    let path = app.path().app_cache_dir().unwrap()
        .join(format!("weekly-{}.html", chrono::Local::now().format("%Y-%m-%d")));
    std::fs::write(&path, &html).map_err(|e| e.to_string())?;
    open::that(&path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}
```

## Settings panel — review preferences

A new "Review" tab in settings:

- Daily notification: on/off, time of day, content level
- Word of day: on/off, time of day (could be multiple times)
- Weekly report: on/off, day of week, time
- Menu bar: on/off, refresh interval (60s default)
- Heatmap depth: thresholds for activity levels

## Acceptance criteria

- [ ] Tray icon appears, shows up-to-date stats label
- [ ] Tray dropdown actions work: open library, open review, open recent
- [ ] Daily notification fires at configured time, shows correct stats
- [ ] Word-of-day window opens with a tracked word
- [ ] Word-of-day buttons correctly transition status; "next" picks a
      different word
- [ ] Word-of-day window has restricted capabilities (verify by
      attempting a forbidden command)
- [ ] Weekly report generates valid standalone HTML (renders correctly
      in Safari/Chrome offline)
- [ ] Heatmap reflects actual daily encounter counts
- [ ] Reports are scoped to the right week (timezone-correct)

## Common Phase 6 problems

**Tray menu doesn't update text:** Tauri's MenuItem text isn't directly
mutable. Workaround: rebuild the menu when stats change.

**Notifications require permission first run:** macOS prompts on first
notification call. Trigger one early (during onboarding) to get the
prompt out of the way.

**Word-of-day window flashes briefly then closes:** if the URL doesn't
match a registered route in your Leptos router, Tauri navigates to a
404 and the auto-close handler runs. Verify the route exists.

**Weekly report dates off by one:** timezone confusion. Always use
`chrono::Local` for "user's day boundaries" and `chrono::Utc` for
storage. Never mix.

**Heatmap shows future days as empty cells:** that's correct, but it
looks weird mid-week. Trim the heatmap to "days completed so far this
week" if you prefer.

## What's next

The app is now a complete tool. Phases 7-10 are extensions:

- Phase 7: native Rust converter (replace AozoraEpub3 jar)
- Phase 8: web importer (URL → EPUB)
- Phase 9: OCR overlay
- Phase 10: local LLM integration

These are independent; pick whichever matters most for your usage.
