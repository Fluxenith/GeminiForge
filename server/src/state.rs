//! Shared server state (the `STATE` dict, `LOCK` and the constants of server.py).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: &str = "1.3.0";
pub const HOST: &str = "127.0.0.1";
/// The operating system, as Gemini is told about it.
pub const OS_NAME: &str = if cfg!(windows) {
    "Windows"
} else if cfg!(target_os = "macos") {
    "macOS"
} else if cfg!(target_os = "linux") {
    "Linux"
} else {
    std::env::consts::OS
};
/// characters of text sent back to Gemini per tool
pub const MAX_OUTPUT: usize = 15000;
/// screenshots are scaled down to this width
pub const SHOT_MAX_WIDTH: u32 = 1366;
/// fail-closed snapshot lifetime in seconds
pub const CAPTURE_TTL: f64 = 60.0;
pub const SKIP_DIRS: &[&str] = &[".git", "node_modules", "__pycache__", ".venv", "venv", "dist", "build", ".next"];
pub const MAX_JOBS: usize = 100;

/// stream read_file above 10 MB (a variable so tests can lower it)
pub static MAX_READ_BYTES: AtomicU64 = AtomicU64::new(10 * 1024 * 1024);
/// cap HTTP POST body at 50 MB
pub static MAX_HTTP_BODY: AtomicUsize = AtomicUsize::new(50 * 1024 * 1024);
/// Gemini gives up on a tool call after about a minute. run waits at most this
/// long (milliseconds here), then leaves the command running as a job.
pub static RUN_WAIT_MS: AtomicU64 = AtomicU64::new(40_000);

/// 0 = fixed waits (exactly like server.py), 1 = fast waits, 2 = not read yet.
static FAST_WAIT: AtomicU8 = AtomicU8::new(2);

/// `GF_FAST_WAIT=1`: wait for the thing a fixed sleep was waiting for (the
/// window coming to the front, the controls or the screen changing) and go on
/// as soon as it has happened. Every such wait keeps the fixed time as its
/// upper limit, so nothing waits longer than the Python version does.
pub fn fast_wait() -> bool {
    match FAST_WAIT.load(Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let on = std::env::var("GF_FAST_WAIT")
                .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes"))
                .unwrap_or(false);
            FAST_WAIT.store(on as u8, Ordering::Relaxed);
            on
        }
    }
}

pub fn set_fast_wait(on: bool) {
    FAST_WAIT.store(on as u8, Ordering::Relaxed);
}

/// Check `done` every `interval` seconds until it is true or `max` seconds
/// have passed. Returns whether it became true.
pub fn poll_until(max: f64, interval: f64, mut done: impl FnMut() -> bool) -> bool {
    let deadline = now() + max;
    loop {
        if done() {
            return true;
        }
        let left = deadline - now();
        if left <= 0.0 {
            return false;
        }
        sleep(interval.min(left));
    }
}

pub fn run_wait() -> f64 {
    RUN_WAIT_MS.load(Ordering::Relaxed) as f64 / 1000.0
}

/// One element remembered from the last ui_elements.
#[derive(Debug, Clone, PartialEq)]
pub struct UiInfo {
    pub hwnd: isize,
    pub title: String,
    pub pid: u32,
    pub ctype: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub t: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
}

/// One OCR line, in pixels of the captured window image.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OcrItem {
    pub text: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub cx: i64,
    pub cy: i64,
    pub words: Vec<OcrWord>,
    /// (start, end, index into words), in code points of `text`
    pub spans: Vec<(usize, usize, usize)>,
}

#[derive(Debug, Clone)]
pub struct OcrEntry {
    pub item: OcrItem,
    pub hwnd: isize,
    pub title: String,
    pub pid: u32,
    pub rx: i64,
    pub ry: i64,
    pub origin: (i32, i32),
    pub capture_id: String,
    pub captured_at: f64,
}

#[derive(Debug, Clone)]
pub struct OcrMeta {
    pub hwnd: isize,
    pub title: String,
    pub pid: u32,
    pub capture_id: String,
    pub captured_at: f64,
    pub lang: String,
    pub method: String,
}

#[derive(Debug, Clone)]
pub struct State {
    pub cwd: String,
    /// real pixels per screenshot pixel
    pub shot_scale: f64,
    /// monitor or window top-left in real pixels
    pub shot_origin: (i32, i32),
    /// (w, h) of the last screenshot, None before the first
    pub shot_size: Option<(u32, u32)>,
    /// target window hwnd if last capture was window-scoped
    pub shot_hwnd: Option<isize>,
    pub shot_capture_id: Option<String>,
    pub shot_time: f64,
    /// element id -> element from the last ui_elements
    pub ui: BTreeMap<i64, UiInfo>,
    /// mark id -> OCR line metadata from the last ocr_window
    pub ocr: BTreeMap<i64, OcrEntry>,
    pub ocr_meta: Option<OcrMeta>,
    pub focus_point: Option<(i32, i32)>,
    pub focus_hwnd: Option<isize>,
}

#[cfg(not(windows))]
fn home_dir() -> String {
    std::env::var("HOME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/".to_string())
}

#[cfg(windows)]
fn home_dir() -> String {
    std::env::var("USERPROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| match (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH")) {
            (Ok(d), Ok(p)) => Some(format!("{d}{p}")),
            _ => None,
        })
        .unwrap_or_else(|| ".".to_string())
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

fn cell() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            cwd: home_dir(),
            shot_scale: 1.0,
            shot_origin: (0, 0),
            shot_size: None,
            shot_hwnd: None,
            shot_capture_id: None,
            shot_time: 0.0,
            ui: BTreeMap::new(),
            ocr: BTreeMap::new(),
            ocr_meta: None,
            focus_point: None,
            focus_hwnd: None,
        })
    })
}

/// Short access to the state. Never call another `with` inside the closure.
pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = cell().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

pub fn cwd() -> String {
    with(|s| s.cwd.clone())
}

/// The global tool lock: one tool runs at a time, as in server.py.
pub fn tool_lock() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `time.time()`
pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn sleep(seconds: f64) {
    if seconds > 0.0 && seconds.is_finite() {
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_until_stops_early_or_at_the_limit() {
        let started = now();
        let mut calls = 0;
        assert!(poll_until(2.0, 0.01, || {
            calls += 1;
            calls >= 3
        }));
        assert!(now() - started < 0.5);
        let started = now();
        assert!(!poll_until(0.15, 0.02, || false));
        let took = now() - started;
        assert!((0.14..0.6).contains(&took), "{took}");
        // the condition is checked once even with no time at all
        assert!(poll_until(0.0, 0.01, || true));
    }
}
