//! Capture, OCR and the click helpers (the Rust side of
//! server/test_win_desktop.py). Nothing is clicked or typed: the live tests
//! only capture a visible window and run OCR on a generated image.
#![cfg(windows)] // the desktop tools are Windows-only so far

mod common;

use common::{call_full, err, serial};
use geminiforge::gui_tools::{take_screenshot, validate_capture_id};
use geminiforge::image::{Img, Labels};
use geminiforge::py::Error;
use geminiforge::state::{self, OcrEntry, OcrItem, OcrMeta};
use geminiforge::win;
use geminiforge::{ocr, ui_tools};
use serde_json::json;

fn visible_window() -> Option<win::Win> {
    win::top_windows().into_iter().find(|w| !w.min && !w.hidden && w.rect.2 >= 200 && w.rect.3 >= 150)
}

#[test]
fn capture_id_staleness_fails_closed() {
    let _g = serial();
    state::with(|s| {
        s.shot_capture_id = Some("cap_11111111".to_string());
        s.shot_time = state::now();
    });
    assert!(validate_capture_id(None, None, None).is_ok());
    assert!(validate_capture_id(Some(&json!("cap_11111111")), None, None).is_ok());
    let mismatch = Error::tool("capture_expired: snapshot id does not match the latest observation; take a fresh screenshot or ocr_window.");
    assert_eq!(validate_capture_id(Some(&json!("cap_22222222")), None, None).unwrap_err(), mismatch);
    state::with(|s| s.shot_time = state::now() - 61.0);
    assert_eq!(
        validate_capture_id(Some(&json!("cap_11111111")), None, None).unwrap_err(),
        Error::tool("capture_expired: snapshot is older than 60s; take a fresh screenshot or ocr_window.")
    );
    // the click tool checks the id before it moves the mouse
    assert_eq!(err("click", json!({"x": 5, "y": 5, "capture_id": "cap_22222222"})), mismatch.http_message());
    state::with(|s| s.shot_capture_id = None);
    assert_eq!(validate_capture_id(Some(&json!("cap_11111111")), None, None).unwrap_err(), mismatch);
}

#[test]
fn click_helpers_refuse_bad_targets() {
    let _g = serial();
    // a window handle that does not exist: nothing is posted, the cursor does not move
    assert_eq!(
        win::post_background_click(0x7fff_fff0, 10, 10, "left", 1).unwrap_err(),
        Error::runtime("Window 2147483632 is no longer valid")
    );
    assert_eq!(
        win::cursor_jump_click(0x7fff_fff0, 10, 10, "left", 1).unwrap_err(),
        Error::runtime("Window 2147483632 is no longer valid")
    );
    assert_eq!(
        win::post_background_click(0x7fff_fff0, 10, 10, "sideways", 1).unwrap_err(),
        Error::value("Invalid mouse button: sideways")
    );
    state::with(|s| s.shot_hwnd = None);
    assert_eq!(
        err("click", json!({"x": 5, "y": 5, "background": true})),
        "background click requires window_id (or a prior window screenshot)."
    );
    assert_eq!(
        err("click", json!({"x": 5, "y": 5, "cursor_jump": true})),
        "cursor_jump click requires window_id (or a prior window screenshot)."
    );
    assert_eq!(
        err("click", json!({"x": 5, "y": 5, "background": true, "window_id": 2147483632u32})),
        "That window is gone or its title changed; run list_windows / ui_elements again."
    );
}

#[test]
fn save_and_restore_foreground_state_is_a_round_trip() {
    let _g = serial();
    let (hwnd, pos) = win::save_foreground_state();
    win::restore_foreground_state(hwnd, pos, Some(hwnd));
    let (hwnd2, pos2) = win::save_foreground_state();
    assert_eq!((hwnd, pos), (hwnd2, pos2));
}

fn ocr_entry(hwnd: isize, text: &str, age: f64) -> OcrEntry {
    OcrEntry {
        item: OcrItem { text: text.to_string(), x: 10, y: 10, w: 80, h: 20, cx: 50, cy: 20, ..Default::default() },
        hwnd,
        title: "Nothing".to_string(),
        pid: 1,
        rx: 150,
        ry: 120,
        origin: (100, 100),
        capture_id: "cap_aaaaaaaa".to_string(),
        captured_at: state::now() - age,
    }
}

#[test]
fn click_text_fails_closed_on_stale_or_foreign_marks() {
    let _g = serial();
    let hwnd = 0x7fff_fff0isize;
    state::with(|s| {
        s.ocr.clear();
        s.ocr.insert(1, ocr_entry(hwnd, "Confirm", 5.0));
        s.ocr.insert(2, ocr_entry(hwnd, "Old mark", 120.0));
        s.ocr_meta = Some(OcrMeta {
            hwnd,
            title: "Nothing".to_string(),
            pid: 1,
            capture_id: "cap_aaaaaaaa".to_string(),
            captured_at: state::now() - 5.0,
            lang: "en-US".to_string(),
            method: "print_window".to_string(),
        });
    });
    assert_eq!(err("click_text", json!({})), "Provide id (from ocr_window) or text.");
    assert_eq!(err("click_text", json!({"id": 9})), "No OCR mark with id 9; run ocr_window first.");
    assert_eq!(
        err("click_text", json!({"id": 1, "window_id": 4})),
        "That OCR mark is not in the given window_id; run ocr_window again."
    );
    assert_eq!(
        err("click_text", json!({"id": 1, "capture_id": "cap_bbbbbbbb"})),
        "capture_expired: snapshot id does not match the latest observation; take a fresh screenshot or ocr_window."
    );
    assert_eq!(
        err("click_text", json!({"id": 2})),
        "capture_expired: OCR snapshot is older than 60s; run ocr_window again."
    );
    // the window the mark belongs to is gone: nothing is clicked
    let gone = "The original window closed or changed; run ui_elements again. Old coordinates were not clicked.";
    assert_eq!(err("click_text", json!({"id": 1, "capture_id": "cap_aaaaaaaa"})), gone);
    assert_eq!(err("click_text", json!({"text": "confirm", "window_id": hwnd as i64})), gone);
    assert_eq!(
        err("click_text", json!({"text": "confirm", "window_id": hwnd as i64, "capture_id": "cap_bbbbbbbb"})),
        "capture_expired: snapshot id does not match the latest observation; take a fresh screenshot or ocr_window."
    );
    assert_eq!(err("click_text", json!({"text": "x", "button": "sideways"})), "Invalid mouse button: sideways");
    state::with(|s| {
        s.ocr.clear();
        s.ocr_meta = None;
    });
}

#[test]
fn zoom_needs_a_screenshot_first_and_a_real_area() {
    let _g = serial();
    state::with(|s| s.shot_size = None);
    assert_eq!(err("zoom", json!({"x1": 0, "y1": 0, "x2": 100, "y2": 100})), "Run screenshot first");
    state::with(|s| {
        s.shot_size = Some((1366, 768));
        s.shot_scale = 1.0;
        s.shot_origin = (0, 0);
    });
    assert_eq!(err("zoom", json!({"x1": 0, "y1": 0, "x2": 5, "y2": 100})), "Area too small");
    assert_eq!(geminiforge::gui_tools::to_shot(100, 50), Some((100, 50)));
    assert_eq!(geminiforge::gui_tools::to_shot(5000, 50), None);
    assert_eq!(geminiforge::gui_tools::to_real(&json!(10.4), &json!("20")).unwrap(), (10, 20));
    state::with(|s| s.shot_size = None);
}

fn is_jpeg(b64: &str) -> bool {
    b64.starts_with("/9j/")
}

#[test]
fn live_monitor_screenshot_sets_state() {
    let _g = serial();
    let (text, image) = call_full("screenshot", json!({"grid": false})).unwrap();
    assert!(text.starts_with("Monitor "), "{text}");
    assert!(text.contains("coordinates are pixels of this image") && text.contains("Switch with screenshot {monitor: N}."));
    assert!(is_jpeg(&image.unwrap()));
    let (size, hwnd, cid, scale) = state::with(|s| (s.shot_size, s.shot_hwnd, s.shot_capture_id.clone(), s.shot_scale));
    let (w, _h) = size.unwrap();
    assert!(w <= 1366 && scale >= 1.0);
    assert_eq!(hwnd, None);
    assert!(text.contains(&format!("capture_id={}", cid.unwrap())));
    let (zoomed, image) = call_full("zoom", json!({"x1": 10, "y1": 10, "x2": 110, "y2": 60})).unwrap();
    assert!(zoomed.starts_with("Zoomed view of (10,10)-(110,60) from the last screenshot."));
    assert!(is_jpeg(&image.unwrap()));
}

#[test]
fn live_window_capture_without_stealing_focus() {
    let _g = serial();
    let Some(w) = visible_window() else {
        eprintln!("skipped: no visible window to capture");
        return;
    };
    let before = win::foreground();
    let (note, image) = take_screenshot(true, None, None, None, Some(&json!(w.hwnd as i64))).unwrap();
    assert_eq!(win::foreground(), before, "capturing a window must not change the foreground window");
    assert!(note.starts_with("Window \""), "{}", &note[..note.len().min(40)]);
    assert!(note.contains(&format!("window_id={}", w.hwnd)));
    assert!(note.contains("method=print_window") || note.contains("method=screen_crop"));
    assert!(is_jpeg(&image));
    let (origin, hwnd, cid) = state::with(|s| (s.shot_origin, s.shot_hwnd, s.shot_capture_id.clone()));
    assert_eq!(hwnd, Some(w.hwnd));
    assert!(note.contains(&format!("capture_id={}", cid.unwrap())));
    // the origin is the visible frame, at or inside the window rectangle
    assert!(origin.0 >= w.rect.0 && origin.1 >= w.rect.1, "{origin:?} {:?}", w.rect);
    let (img, vis, method) = win::capture_window_image(w.hwnd).unwrap();
    assert_eq!((img.w as i32, img.h as i32), (vis.2, vis.3));
    assert!(method == "print_window" || method == "screen_crop");
}

#[test]
fn live_ocr_reads_generated_text() {
    let _g = serial();
    let mut img = Img::new(640, 120);
    for b in img.rgb.iter_mut() {
        *b = 255;
    }
    Labels::with_height(48).draw(&mut img, 20, 30, "HELLO OCR 12345", (0, 0, 0, 255));
    let started = std::time::Instant::now();
    let (lang, items) = match ocr::ocr_image(&img, 15.0) {
        Ok(r) => r,
        Err(e) if e.py_str().contains("No Windows OCR language pack") => {
            eprintln!("skipped: {}", e.py_str());
            return;
        }
        Err(e) => panic!("{}", e.http_message()),
    };
    assert!(!lang.is_empty());
    let all: String = items.iter().map(|it| it.text.to_uppercase()).collect::<Vec<_>>().join(" ");
    assert!(all.contains("HELLO") && all.contains("12345"), "{all}");
    let line = items.iter().find(|it| it.text.to_uppercase().contains("HELLO")).unwrap();
    // the box of a substring is tighter than the line and inside it
    let (bx, by, bw, bh) = ocr::find_substring_box(line, "hello");
    assert!(bw < line.w && bx >= line.x && by >= line.y && bh <= line.h, "{:?}", (bx, by, bw, bh));
    // a second call reuses the engine on the same worker thread
    let (_, again) = ocr::ocr_image(&img, 15.0).unwrap();
    assert_eq!(again, items);
    assert!(started.elapsed().as_secs_f64() < 15.0);
    let marked = ocr::draw_som_overlay(&img, &[(1, line)], 1.0);
    assert_ne!(marked, img);
}

#[test]
fn batch_tool_list_is_the_gui_subset() {
    for name in ui_tools::BATCH_GUI_TOOLS {
        assert!(geminiforge::tools::find(name).is_some(), "{name}");
    }
    for name in ["run", "write_file", "open", "screenshot", "gui_batch", "click_elements", "move", "drag"] {
        assert!(!ui_tools::BATCH_GUI_TOOLS.contains(&name), "{name}");
    }
}

#[test]
fn fast_wait_before_the_screenshot_never_exceeds_the_fixed_wait() {
    let _g = serial();
    let before = geminiforge::gui_tools::screen_marks();
    let started = std::time::Instant::now();
    geminiforge::gui_tools::settle_screen(&before, None, None, 0.4);
    let took = started.elapsed().as_secs_f64();
    assert!(took < 0.4 + 0.35, "{took}");
    // a window-only screenshot keeps the fixed wait
    let started = std::time::Instant::now();
    geminiforge::gui_tools::settle_screen(&before, None, Some(1), 0.2);
    assert!(started.elapsed().as_secs_f64() >= 0.19);
}

#[test]
fn fast_wait_is_off_unless_asked_for() {
    let _g = serial();
    if std::env::var("GF_FAST_WAIT").is_err() {
        assert!(!state::fast_wait());
    }
    state::set_fast_wait(true);
    assert!(state::fast_wait());
    state::set_fast_wait(false);
    assert!(!state::fast_wait());
}
