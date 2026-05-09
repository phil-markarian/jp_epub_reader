# Phase 9 — OCR Overlay

**Goal:** press a hotkey, capture a screen region (or full screen), OCR
it, show dictionary popups under the cursor for whatever text is hovered.
meikipop's UX with our dictionary engine and vocab tracking behind it.

**Time:** 2 weekends.

## Architecture

Three pieces working together:

```
Global hotkey (Shift+Cmd+J or similar)
  ↓
Region selector window (transparent, fullscreen, click-drag rectangle)
  ↓
Screen capture (CGWindowListCreateImage / ScreenCaptureKit)
  ↓
OCR (Apple Vision framework — VNRecognizeTextRequest)
  ↓
Result: text strings with bounding boxes
  ↓
Overlay window (transparent, always-on-top, positioned over original area)
  ↓ for each OCR'd text region:
   draw invisible hit boxes
   on hover: call dict::lookup
   show LookupPopup component (same as Phase 5)
   on action: write to vocab DB (same path as Phase 5)
```

## Crate

```
crates/jp-ocr/
├─ Cargo.toml
├─ src/
│  ├─ lib.rs                   # public API
│  ├─ capture.rs               # screen capture wrapper
│  ├─ vision.rs                # Apple Vision OCR (macOS)
│  ├─ tesseract.rs             # fallback for Linux/Windows (later)
│  └─ types.rs                 # OcrResult, OcrLine, BBox
```

## Apple Vision integration

The big advantage on macOS: Vision framework has built-in Japanese OCR
including vertical text. Free, accurate, fast, fully local.

Bindings via `objc2`:

```toml
[dependencies]
objc2 = "0.5"
objc2-foundation = "0.2"
objc2-core-foundation = "0.2"
objc2-core-graphics = "0.2"
objc2-vision = "0.2"
objc2-core-image = "0.2"
```

```rust
// crates/jp-ocr/src/vision.rs

use objc2::rc::Retained;
use objc2_foundation::{NSString, NSArray};
use objc2_vision::{
    VNRecognizeTextRequest, VNRequestTextRecognitionLevel,
    VNImageRequestHandler, VNRecognizedTextObservation,
};
use objc2_core_image::CIImage;

pub fn recognize(image_data: &[u8]) -> Result<OcrResult> {
    autoreleasepool(|_| {
        // 1. Create CIImage from bytes
        let ci_image = CIImage::imageWithData(/* NSData from bytes */);

        // 2. Configure request: Japanese, accurate (not fast) recognition
        let request = unsafe {
            let req = VNRecognizeTextRequest::new();
            req.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            req.setRecognitionLanguages(
                &NSArray::from_vec(vec![NSString::from_str("ja-JP")])
            );
            req.setUsesLanguageCorrection(true);
            req
        };

        // 3. Execute
        let handler = unsafe { VNImageRequestHandler::initWithCIImage_options(/* ... */) };
        unsafe {
            handler.performRequests_error(&NSArray::from_vec(vec![&*request]))?;
        }

        // 4. Extract observations: text + bounding boxes
        let observations = unsafe { request.results() };
        let mut lines = Vec::new();
        for obs in observations.iter() {
            let candidates = unsafe { obs.topCandidates(1) };
            if let Some(top) = candidates.first() {
                let text = unsafe { top.string() };
                let bbox = unsafe { obs.boundingBox() };
                lines.push(OcrLine {
                    text: text.to_string(),
                    bbox: BBox::from_normalized(bbox, image_width, image_height),
                });
            }
        }

        Ok(OcrResult { lines })
    })
}
```

(The objc2 syntax is verbose; this is sketch-level. Real implementation
will need careful memory management with `autoreleasepool` and explicit
`Retained<T>` wrappers.)

Vertical text: Vision auto-detects orientation. For mixed
horizontal/vertical, results are returned in reading order regardless.

## Screen capture

Use `screencapturekit-rs` (modern macOS, supports retina) or
`core-graphics`'s `CGWindowListCreateImage` (older but simpler).

```rust
// crates/jp-ocr/src/capture.rs

pub async fn capture_region(rect: ScreenRect) -> Result<Vec<u8>> {
    // Returns PNG bytes of the captured region.
    // First call triggers macOS Screen Recording permission prompt.
    todo!()
}

pub async fn capture_full_screen() -> Result<Vec<u8>> {
    todo!()
}
```

macOS permission: Screen Recording. Add to Info.plist:

```xml
<key>NSScreenCaptureDescription</key>
<string>aozora-tool needs screen recording to OCR Japanese text from
games, manga, and videos.</string>
```

The system prompts on first capture attempt. If denied, fall back to
clipboard-paste workflow (user manually screenshots, pastes into a
field).

## Region selector window

A transparent fullscreen Tauri window for click-drag rectangle
selection.

```rust
pub async fn show_region_selector(app: &AppHandle) -> Result<ScreenRect> {
    let window = WebviewWindowBuilder::new(
        app,
        "ocr-region-selector",
        WebviewUrl::App("/ocr/select-region".into()),
    )
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .fullscreen(true)
    .resizable(false)
    .build()?;

    // Wait for selection event from frontend
    let (tx, rx) = oneshot::channel();
    window.listen("ocr:region-selected", move |event| {
        let rect: ScreenRect = serde_json::from_str(event.payload()).unwrap();
        let _ = tx.send(rect);
    });

    let rect = rx.await?;
    window.close()?;
    Ok(rect)
}
```

The frontend route `/ocr/select-region` is a Leptos page with a single
fullscreen div that captures mousedown/mousemove/mouseup, draws a
selection rectangle, and emits the final coordinates on mouseup.

## Overlay window

After OCR completes, an overlay window positioned exactly over the
captured region shows interactive hit zones for each detected text
line.

```rust
pub async fn show_overlay(
    app: &AppHandle,
    rect: ScreenRect,
    ocr: OcrResult,
) -> Result<()> {
    let label = "ocr-overlay";
    let window = WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::App(format!("/ocr/overlay?session_id={}", session_id).into()),
    )
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .position(rect.x, rect.y)
    .inner_size(rect.width, rect.height)
    .skip_taskbar(true)
    .build()?;

    // Frontend pulls OCR result by session_id
    Ok(())
}
```

The overlay's frontend renders one transparent div per OCR line at the
correct bounding box. Hovering a div fires `lookup`, which:

1. Looks up the word at the cursor position in the OCR'd line
2. Records an encounter with `source_type='ocr', source_ref='{session_id}'`
3. Shows the same `LookupPopup` from Phase 5
4. The 4 buttons (track / mine / known / ignore) work the same way

So OCR'd words flow into the same vocab DB as reader-sourced words.
Stats include them, mining works, review surfaces show them.

## Global hotkey

```rust
use tauri_plugin_global_shortcut::{Code, Modifiers, GlobalShortcutExt};

pub fn register_hotkey(app: &AppHandle) -> Result<()> {
    app.global_shortcut().register("Shift+Cmd+J")?;
    app.listen_global("global-shortcut", move |event| {
        let app = event.app_handle().clone();
        tokio::spawn(async move {
            let rect = show_region_selector(&app).await?;
            let png = capture_region(rect).await?;
            let ocr = recognize(&png)?;
            show_overlay(&app, rect, ocr).await?;
            Ok::<(), anyhow::Error>(())
        });
    });
    Ok(())
}
```

User-configurable hotkey via settings.

## OCR session tracking

Each invocation gets a session_id. Encounters from that session share
the source_ref:

```sql
INSERT INTO source (type, ref, title, first_opened_at, last_opened_at)
VALUES ('ocr', '{session_id}', 'OCR session 2026-05-08 14:32', ?, ?)
```

The "title" is the timestamp by default. UI lets the user rename
sessions ("Steins;Gate Episode 12", "this manga page", etc.) — useful
for the source breakdown in the weekly report.

## Capability declaration

```json
{
  "identifier": "ocr",
  "windows": ["ocr-overlay", "ocr-region-selector"],
  "permissions": [
    "core:default",
    {
      "identifier": "http:default",
      "allow": [{ "url": "http://127.0.0.1:8765/*" }]
    }
  ]
}
```

OCR overlay windows can talk to AnkiConnect (for mining) but not the
broader filesystem. Vocab DB access goes through narrow commands.

## Cross-platform note

For Linux/Windows, Apple Vision isn't available. Options:

- **Tesseract** (`tesseract-rs` or via subprocess) — works but mediocre
  Japanese, especially vertical
- **Manga-OCR** (Python, ML-based) — best Japanese accuracy, requires
  Python + PyTorch runtime
- **owocr** (the meikipop dependency) — handles backend selection
- **Cloud OCR** — out of scope (we're local-only)

For v1 ship macOS-only OCR. Add other platforms later if you actually
need them.

## Acceptance criteria

- [ ] Hotkey opens region selector overlay
- [ ] Region selection captures the correct screen area on retina
      displays
- [ ] Vision OCR recognizes both horizontal and vertical Japanese text
- [ ] Overlay window shows interactive hit boxes over original positions
- [ ] Hovering a word fires the lookup popup
- [ ] Mining a word from OCR creates a card with `Source: OCR
      session XXX` template field
- [ ] Vocab DB shows OCR encounters with `source_type='ocr'`
- [ ] Encounters from the same session share a source_ref
- [ ] User can rename OCR sessions to meaningful titles
- [ ] Permission denied gracefully — falls back to clipboard-paste
- [ ] Hotkey can be reconfigured via settings

## Common Phase 9 problems

**Vision OCR misreads vertical text:** ensure
`automaticallyDetectsLanguage` is true and
`recognitionLanguages` includes "ja-JP". Test with manga page screenshots.

**Overlay hit boxes don't align with OCR text:** Vision returns
normalized bounding boxes (0-1 range), not pixel coordinates. Convert
to pixels using the captured image's actual dimensions, then position
in the overlay using the original screen rect.

**Multiple monitors:** screen capture gets weird across monitors.
First version: support primary monitor only, document the limitation.

**Performance: capture + OCR takes >1s:** Vision is fast (~200-400ms
on M-series); the bottleneck is usually the screen capture path. Use
`screencapturekit-rs` for fast capture; `CGWindowListCreateImage` is
slower.

**Overlay windows don't always close cleanly:** track sessions in
state; clean up on app quit.

## What's next

Phase 10: local LLM integration. The OCR overlay benefits from LLM
augmentation (grammar breakdown of game dialogue), so Phase 9 + 10
combine naturally.
