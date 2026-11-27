//! Window targeting and the fail-closed rules of the UI tools (the Rust side of
//! server/test_ui_tools.py). No test here clicks or types: the only live calls
//! read an already visible window.
#![cfg(windows)] // the desktop tools are Windows-only so far

mod common;

use common::{call, err, serial};
use geminiforge::py::Error;
use geminiforge::state::{self, UiInfo};
use geminiforge::tools;
use geminiforge::win::{self, Win};
use serde_json::json;

fn win_of(hwnd: isize, title: &str, hidden: bool) -> Win {
    Win { hwnd, title: title.to_string(), min: false, hidden, rect: (0, 0, 800, 600) }
}

fn fake_windows() -> Vec<Win> {
    vec![
        win_of(10, "Calculator automation test - Google Chrome", false),
        win_of(11, "Calculator", true),
        win_of(12, "Calculator", false),
        win_of(13, "calculator notes", false),
    ]
}

fn info(hwnd: isize, ctype: &str, name: &str) -> UiInfo {
    UiInfo { hwnd, title: "Calculator".into(), pid: 30, ctype: ctype.into(), name: name.into(), x: 10, y: 10 }
}

fn seed_ui() {
    state::with(|s| {
        s.ui.clear();
        s.ui.insert(1, info(12, "ButtonControl", "Seven"));
        s.ui.insert(2, info(12, "ButtonControl", "Plus"));
        s.ui.insert(3, info(12, "ButtonControl", "Equals"));
    });
}

#[test]
fn exact_window_wins_over_browser_tab() {
    let w = win::pick_window(fake_windows, Some(&json!("Calculator")), None).unwrap();
    // exact title first, and a visible window before a hidden one with the same title
    assert_eq!(w.hwnd, 12);
    let w = win::pick_window(fake_windows, Some(&json!("calculator n")), None).unwrap();
    assert_eq!(w.hwnd, 13);
    let w = win::pick_window(fake_windows, Some(&json!("automation")), None).unwrap();
    assert_eq!(w.hwnd, 10);
}

#[test]
fn window_id_disambiguates_identical_titles() {
    assert_eq!(win::pick_window(fake_windows, None, Some(&json!(11))).unwrap().hwnd, 11);
    assert_eq!(win::pick_window(fake_windows, Some(&json!("CALCULATOR")), Some(&json!("12"))).unwrap().hwnd, 12);
    let gone = Error::tool("That window is gone or its title changed; run list_windows / ui_elements again.");
    assert_eq!(win::pick_window(fake_windows, None, Some(&json!(99))).unwrap_err(), gone);
    assert_eq!(win::pick_window(fake_windows, Some(&json!("Notepad")), Some(&json!(12))).unwrap_err(), gone);
}

#[test]
fn empty_window_selector_is_rejected() {
    let need = Error::tool("Provide a window title or window_id.");
    assert_eq!(win::pick_window(fake_windows, None, None).unwrap_err(), need);
    assert_eq!(win::pick_window(fake_windows, Some(&json!("   ")), None).unwrap_err(), need);
    assert_eq!(
        win::pick_window(fake_windows, Some(&json!("Notepad")), None).unwrap_err(),
        Error::tool("No window title contains \"Notepad\". Use list_windows to see open windows.")
    );
}

#[test]
fn failed_observation_clears_cached_elements() {
    let _g = serial();
    seed_ui();
    assert!(call("ui_elements", json!({"window": "zz-no-such-window-zz"})).unwrap_err().is_tool());
    assert!(state::with(|s| s.ui.is_empty()));
    assert_eq!(err("click_element", json!({"id": 1})), "No element with id 1; run ui_elements first.");
}

#[test]
fn window_title_or_process_change_rejects_cached_control() {
    let _g = serial();
    seed_ui();
    // hwnd 12 is not a real window: the remembered element must not be clicked
    let changed = "The original window closed or changed; run ui_elements again. Old coordinates were not clicked.";
    assert_eq!(err("click_element", json!({"id": 1, "screenshot": false})), changed);
    assert_eq!(err("type_element", json!({"text": "x", "id": 1, "screenshot": false})), changed);
    assert_eq!(win::validate_ui_window(12, "Calculator", 30).unwrap_err(), Error::tool(changed));
    state::with(|s| s.ui.clear());
}

#[test]
fn batch_checks_entire_plan_before_first_click() {
    let _g = serial();
    seed_ui();
    // an unknown name anywhere in the plan stops it before anything is clicked
    assert_eq!(
        err("click_elements", json!({"window_id": 12, "names": ["Seven", "Minus", "Equals"], "screenshot": false})),
        "No element named \"Minus\" in the last ui_elements result."
    );
    assert_eq!(
        err("click_elements", json!({"window_id": 99, "names": ["Seven"], "screenshot": false})),
        "The element is not in the given window_id; run ui_elements again."
    );
    state::with(|s| s.ui.clear());
}

#[test]
fn batch_has_a_finite_limit() {
    let _g = serial();
    let bad = "names must be an array of 1-16 button names.";
    let many: Vec<&str> = vec!["Seven"; 17];
    assert_eq!(err("click_elements", json!({"window_id": 12, "names": many})), bad);
    assert_eq!(err("click_elements", json!({"window_id": 12, "names": []})), bad);
    assert_eq!(err("click_elements", json!({"window_id": 12, "names": ["Seven", ""]})), bad);
    assert_eq!(err("click_elements", json!({"window_id": 12, "names": ["Seven", 7]})), bad);
    assert_eq!(err("click_elements", json!({"window_id": 12, "names": "Seven"})), bad);
}

#[test]
fn ui_calls_share_one_living_thread_across_request_workers() {
    let ids: Vec<(std::thread::ThreadId, Option<String>)> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                tools::run_on_ui(|| {
                    let t = std::thread::current();
                    (t.id(), t.name().map(|n| n.to_string()))
                })
                .unwrap()
            })
        })
        .map(|h| h.join().unwrap())
        .collect();
    assert!(ids.iter().all(|i| i == &ids[0]), "{ids:?}");
    assert_eq!(ids[0].1.as_deref(), Some("GeminiForgeUI_0"));
    assert_ne!(ids[0].0, std::thread::current().id());
    // every UI Automation tool is routed there
    for name in ["ui_elements", "read_window", "click_element", "click_elements", "type_element", "focus_window", "scroll", "wait_for", "gui_batch"] {
        assert!(tools::UI_TOOLS.contains(&name), "{name}");
    }
}

#[test]
fn scroll_and_wait_for_validate_their_arguments() {
    let _g = serial();
    assert_eq!(err("scroll", json!({})), "Provide amount or percent.");
    assert_eq!(err("scroll", json!({"amount": 0})), "amount must be non-zero (positive = up, negative = down).");
    assert_eq!(err("scroll", json!({"amount": "x"})), "ValueError: invalid literal for int() with base 10: 'x'");
    assert_eq!(err("wait_for", json!({})), "Provide at least one of window, window_id, text, or element.");
    assert_eq!(
        err("wait_for", json!({"window": "zz-no-such-window-zz", "timeout": 0.3, "interval": 0.1})),
        "Timed out after 0.3s waiting for window \"zz-no-such-window-zz\" to appear."
    );
    let gone = call("wait_for", json!({"window": "zz-no-such-window-zz", "gone": true})).unwrap();
    assert!(gone.starts_with("Condition met in 0.0s: window \"zz-no-such-window-zz\" disappeared."), "{gone}");
    assert_eq!(
        err("wait_for", json!({"window": "zz-no-such-window-zz", "text": "x", "timeout": 0.2})),
        "Timed out after 0.2s waiting for text \"x\" to appear."
    );
}

#[test]
fn gui_batch_validation_and_stop_on_error() {
    let _g = serial();
    let steps = "steps must be an array of 1-16 step objects.";
    assert_eq!(err("gui_batch", json!({"steps": []})), steps);
    assert_eq!(err("gui_batch", json!({"steps": "click"})), steps);
    assert_eq!(err("gui_batch", json!({"steps": vec![json!({"action": "wait"}); 17]})), steps);
    assert_eq!(err("gui_batch", json!({"steps": ["click"]})), "Step 1 must be a JSON object.");
    assert_eq!(
        err("gui_batch", json!({"steps": [{"action": "run", "command": "x"}]})),
        "Step 1 has unsupported action 'run'. Allowed: click, click_element, click_text, double_click, focus_window, key, ocr_window, read_window, right_click, scroll, type, type_element, ui_elements, wait, wait_for"
    );
    assert!(err("gui_batch", json!({"steps": [{"args": {}}]})).starts_with("Step 1 has unsupported action None."));
    assert_eq!(err("gui_batch", json!({"steps": [{"action": "wait", "args": 5}]})), "Step 1 args must be a JSON object.");

    // steps run in order and stop at the first failure, which is reported with its position
    let out = call(
        "gui_batch",
        json!({"screenshot": false, "steps": [{"action": "wait", "seconds": 0.01}, {"tool": "wait", "args": {"seconds": 0}}]}),
    )
    .unwrap();
    assert_eq!(out, "Completed 2 step(s):\n1. wait: Waited 0.01 seconds\n2. wait: Waited 0 seconds");
    assert_eq!(
        err("gui_batch", json!({"screenshot": false, "steps": [{"action": "wait", "seconds": 0}, {"action": "key", "keys": "+"}, {"action": "wait"}]})),
        "gui_batch stopped at step 2/3 (key); 1 step(s) completed: Provide at least one key."
    );
    assert_eq!(
        err("gui_batch", json!({"screenshot": false, "steps": [{"action": "wait", "secs": 1}]})),
        "gui_batch stopped at step 1/1 (wait); 0 step(s) completed: t_wait() got an unexpected keyword argument 'secs'"
    );
}

/// A visible, non-minimized window to read (never activated, never clicked).
fn readable_window() -> Option<Win> {
    win::top_windows().into_iter().find(|w| !w.min && !w.hidden && w.rect.2 >= 200 && w.rect.3 >= 150)
}

#[test]
fn live_ui_elements_and_read_window_only_read() {
    let _g = serial();
    let Some(w) = readable_window() else {
        eprintln!("skipped: no visible window to read");
        return;
    };
    let before = win::foreground();
    let listed = call("ui_elements", json!({"window_id": w.hwnd, "max": 5})).unwrap();
    assert!(listed.starts_with("Elements in window \""), "{}", listed.lines().next().unwrap_or(""));
    assert!(listed.lines().next().unwrap().contains(&format!("window_id={}", w.hwnd)));
    assert!(state::with(|s| s.ui.len()) <= 5);
    let read = call("read_window", json!({"window_id": w.hwnd, "max_chars": 200})).unwrap();
    let head = read.lines().next().unwrap().to_string();
    assert!(head.starts_with("Window \"") && head.contains(" [chars 0-"), "{head}");
    assert_eq!(win::foreground(), before, "reading a window must not change the foreground window");
    state::with(|s| s.ui.clear());
}
