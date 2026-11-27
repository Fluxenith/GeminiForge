//! The tool table and `call_tool` (argument binding, the global lock, the
//! single UI thread and the screenshot taken after screen actions).

use crate::py::{self, Error, Res};
use crate::state::{self, MAX_OUTPUT};
use serde_json::{Map, Value};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

/// What a tool returns: text, and a base64 JPEG for the tools that produce one.
pub struct ToolOut {
    pub text: String,
    pub image: Option<String>,
}

impl From<String> for ToolOut {
    fn from(text: String) -> ToolOut {
        ToolOut { text, image: None }
    }
}

impl From<&str> for ToolOut {
    fn from(text: &str) -> ToolOut {
        ToolOut { text: text.to_string(), image: None }
    }
}

static NULL: Value = Value::Null;

/// Keyword arguments of one tool call.
pub struct Args<'a> {
    map: &'a Map<String, Value>,
}

impl<'a> Args<'a> {
    pub fn new(map: &'a Map<String, Value>) -> Args<'a> {
        Args { map }
    }
    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key)
    }
    /// A required argument (presence was checked when the call was bound).
    pub fn req(&self, key: &str) -> &'a Value {
        self.map.get(key).unwrap_or(&NULL)
    }
    /// The argument, or the Python default when it was not passed.
    pub fn or(&self, key: &str, default: Value) -> Value {
        self.map.get(key).cloned().unwrap_or(default)
    }
    pub fn truthy(&self, key: &str, default: bool) -> bool {
        self.map.get(key).map(py::truthy).unwrap_or(default)
    }
    /// For parameters that default to None: `None` when absent or null.
    pub fn opt(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key).filter(|v| !v.is_null())
    }
    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }
}

pub type ToolFn = fn(&Args) -> Res<ToolOut>;

pub struct ToolDef {
    pub name: &'static str,
    pub func: ToolFn,
    /// (parameter name, required) in declaration order
    pub params: &'static [(&'static str, bool)],
}

macro_rules! tool {
    ($name:literal, $func:path, [$( $p:literal $(: $req:ident)? ),* $(,)?]) => {
        ToolDef { name: $name, func: $func, params: &[$( ($p, tool!(@req $($req)?)) ),*] }
    };
    (@req req) => { true };
    (@req) => { false };
}

use crate::{fs_tools as f, gui_tools as g, ocr_tools as o, proc_tools as p, ui_tools as u};

/// `TOOLS`, in the order server.py lists them (the order /health reports).
pub static TOOLS: &[ToolDef] = &[
    tool!("cd", f::t_cd, ["path": req]),
    tool!("list_dir", f::t_list_dir, ["path", "depth", "directories_only"]),
    tool!("read_file", f::t_read_file, ["path": req, "offset", "limit"]),
    tool!("read_files", f::t_read_files, ["paths": req, "limit"]),
    tool!("write_file", f::t_write_file, ["path": req, "content": req]),
    tool!("edit_file", f::t_edit_file, ["path": req, "old": req, "new": req, "replace_all"]),
    tool!("edit_files", f::t_edit_files, ["edits": req]),
    tool!("search", f::t_search, ["pattern": req, "path", "glob", "ignore_case"]),
    tool!("find_files", f::t_find_files, ["pattern": req, "path"]),
    tool!("run", p::t_run, ["command": req, "cwd", "timeout"]),
    tool!("start_process", p::t_start_process, ["command": req, "cwd"]),
    tool!("job_status", p::t_job_status, ["id", "seconds"]),
    tool!("stop_job", p::t_stop_job, ["id": req]),
    tool!("open", p::t_open, ["target": req]),
    tool!("screenshot", g::t_screenshot, ["grid", "monitor", "window", "window_id"]),
    tool!("click", g::t_click, ["x": req, "y": req, "button", "clicks", "window_id", "background", "cursor_jump", "restore_focus", "capture_id"]),
    tool!("double_click", g::t_double_click, ["x": req, "y": req]),
    tool!("right_click", g::t_right_click, ["x": req, "y": req]),
    tool!("move", g::t_move, ["x": req, "y": req]),
    tool!("drag", g::t_drag, ["x1": req, "y1": req, "x2": req, "y2": req]),
    tool!("type", g::t_type, ["text": req, "enter", "window_id", "restore_focus"]),
    tool!("key", g::t_key, ["keys": req, "window_id", "restore_focus"]),
    tool!("scroll", u::t_scroll, ["amount", "x", "y", "window_id", "percent"]),
    tool!("wait", p::t_wait, ["seconds"]),
    tool!("clipboard_get", g::t_clipboard_get, []),
    tool!("clipboard_set", g::t_clipboard_set, ["text": req]),
    tool!("list_windows", g::t_list_windows, []),
    tool!("focus_window", g::t_focus_window, ["title", "window_id"]),
    tool!("ui_elements", u::t_ui_elements, ["window", "filter", "max", "include_text", "window_id", "include_offscreen", "offset"]),
    tool!("read_window", u::t_read_window, ["window", "window_id", "include_offscreen", "offset", "max_chars"]),
    tool!("click_element", u::t_click_element, ["id", "name", "button", "clicks", "window_id", "restore_focus"]),
    tool!("click_elements", u::t_click_elements, ["window_id": req, "names": req]),
    tool!("type_element", u::t_type_element, ["text": req, "id", "name", "window_id", "clear", "enter", "restore_focus"]),
    tool!("ocr_window", o::t_ocr_window, ["window", "window_id", "filter", "overlay", "max"]),
    tool!("click_text", o::t_click_text, ["text", "id", "window_id", "button", "clicks", "background", "cursor_jump", "restore_focus", "capture_id"]),
    tool!("wait_for", u::t_wait_for, ["window", "window_id", "text", "element", "gone", "use_ocr", "timeout", "interval"]),
    tool!("gui_batch", u::t_gui_batch, ["steps": req, "window_id", "restore_focus"]),
    tool!("zoom", g::t_zoom, ["x1": req, "y1": req, "x2": req, "y2": req]),
    tool!("list_backups", f::t_list_backups, ["limit", "path"]),
    tool!("restore_backup", f::t_restore_backup, ["backup_id": req]),
];

/// These change what is on screen, so a fresh screenshot is returned after them.
pub const SCREEN_ACTIONS: &[&str] = &[
    "click", "double_click", "right_click", "move", "drag", "type", "key", "scroll", "open",
    "click_element", "click_elements", "type_element", "click_text", "gui_batch", "focus_window",
];

/// Tools that use UI Automation: they all run on the one UI thread.
pub const UI_TOOLS: &[&str] = &[
    "ui_elements", "read_window", "click_element", "click_elements", "type_element",
    "focus_window", "scroll", "wait_for", "gui_batch",
];

/// The desktop tools only Windows has so far (mouse and keyboard, windows,
/// UI elements, OCR). screenshot works everywhere.
pub const DESKTOP_TOOLS: &[&str] = &[
    "click", "double_click", "right_click", "move", "drag", "type", "key", "scroll",
    "list_windows", "focus_window", "ui_elements", "read_window", "click_element", "click_elements",
    "type_element", "ocr_window", "click_text", "wait_for", "gui_batch", "zoom",
];

/// Whether this system has the tool (every tool on Windows).
pub fn available(name: &str) -> bool {
    cfg!(windows) || !DESKTOP_TOOLS.contains(&name)
}

/// The error for a tool this system does not have yet.
pub fn unavailable(name: &str) -> Error {
    Error::tool(format!(
        "{name} is not available on {} yet. GeminiForge on this computer has these tools: {}.",
        state::OS_NAME,
        tool_names().join(", ")
    ))
}

pub fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

/// The tools this system has, in table order.
pub fn tool_names() -> Vec<&'static str> {
    TOOLS.iter().map(|t| t.name).filter(|n| available(n)).collect()
}

/// `text[:half] + "...[N characters truncated]..." + text[-half:]`
pub fn clip(text: &str) -> String {
    let n = py::len(text);
    if n <= MAX_OUTPUT {
        return text.to_string();
    }
    let half = MAX_OUTPUT / 2;
    format!(
        "{}\n\n...[{} characters truncated]...\n\n{}",
        py::prefix(text, half),
        n - MAX_OUTPUT,
        py::suffix(text, half)
    )
}

fn quoted_list(names: &[&str]) -> String {
    let q: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
    match q.len() {
        0 => String::new(),
        1 => q[0].clone(),
        2 => format!("{} and {}", q[0], q[1]),
        n => format!("{}, and {}", q[..n - 1].join(", "), q[n - 1]),
    }
}

/// The `TypeError` Python raises when `fn(**args)` cannot be bound.
pub fn check_args(def: &ToolDef, args: &Map<String, Value>) -> Res<()> {
    let names: Vec<&str> = def.params.iter().map(|(n, _)| *n).collect();
    for key in args.keys() {
        if !names.contains(&key.as_str()) {
            let hint = match py::suggest(&names, key) {
                Some(s) => format!(". Did you mean '{s}'?"),
                None => String::new(),
            };
            return Err(Error::type_err(format!(
                "t_{}() got an unexpected keyword argument '{key}'{hint}",
                def.name
            )));
        }
    }
    let missing: Vec<&str> =
        def.params.iter().filter(|(n, req)| *req && !args.contains_key(*n)).map(|(n, _)| *n).collect();
    if !missing.is_empty() {
        return Err(Error::type_err(format!(
            "t_{}() missing {} required positional argument{}: {}",
            def.name,
            missing.len(),
            if missing.len() == 1 { "" } else { "s" },
            quoted_list(&missing)
        )));
    }
    Ok(())
}

/// Bind and run a tool function the way `fn(**args)` does (no lock, no screenshot).
pub fn invoke(def: &ToolDef, args: &Map<String, Value>) -> Res<ToolOut> {
    check_args(def, args)?;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (def.func)(&Args::new(args)))) {
        Ok(res) => res,
        Err(_) => Err(Error::runtime(format!("internal error in {}", def.name))),
    }
}

type UiJob = Box<dyn FnOnce() + Send + 'static>;

fn ui_sender() -> &'static Mutex<mpsc::Sender<UiJob>> {
    static TX: OnceLock<Mutex<mpsc::Sender<UiJob>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<UiJob>();
        std::thread::Builder::new()
            .name("GeminiForgeUI_0".to_string())
            .spawn(move || {
                #[cfg(windows)]
                crate::uia::init_thread();
                for job in rx {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            })
            .expect("cannot start the UI thread");
        Mutex::new(tx)
    })
}

/// Run `f` on the one living UI thread and wait for it (`UI_EXECUTOR.submit(...).result()`).
pub fn run_on_ui<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Res<R> {
    let (tx, rx) = mpsc::channel::<R>();
    let job: UiJob = Box::new(move || {
        let _ = tx.send(f());
    });
    ui_sender()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send(job)
        .map_err(|_| Error::runtime("UI thread is not running"))?;
    rx.recv().map_err(|_| Error::runtime("UI thread failed"))
}

/// `call_tool(name, args)`: returns (text, image).
pub fn call_tool(name: &Value, args: Map<String, Value>) -> Res<(String, Option<String>)> {
    if matches!(name, Value::Array(_) | Value::Object(_)) {
        return Err(Error::type_err(format!("unhashable type: '{}'", py::type_name(name))));
    }
    if let Some(n) = name.as_str().filter(|n| find(n).is_some() && !available(n)) {
        return Err(unavailable(n));
    }
    let is_ui = name.as_str().map(|n| UI_TOOLS.contains(&n)).unwrap_or(false);
    if is_ui {
        let name = name.clone();
        return run_on_ui(move || call_tool_inner(&name, args))?;
    }
    call_tool_inner(name, args)
}

fn call_tool_inner(name: &Value, mut args: Map<String, Value>) -> Res<(String, Option<String>)> {
    let Some(def) = name.as_str().and_then(find) else {
        return Err(Error::tool(format!(
            "Unknown tool: {}. Available: {}",
            py::py_str(name),
            tool_names().join(", ")
        )));
    };
    let want_shot = args.shift_remove("screenshot").map(|v| py::truthy(&v)).unwrap_or(true);
    let _guard = state::tool_lock();
    state::with(|s| {
        s.focus_point = None;
        s.focus_hwnd = None;
    });
    // only Windows takes screenshots so far
    let shot_after = cfg!(windows) && SCREEN_ACTIONS.contains(&def.name) && want_shot;
    // with fast waits: what the screen looks like before the action
    let before = (shot_after && state::fast_wait()).then(g::screen_marks);
    let out = invoke(def, &args)?;
    let (mut text, mut image) = (out.text, out.image);
    if image.is_none() && shot_after {
        // A click that did not bring its window to the front is checked on
        // that window, which other windows may be covering on the monitor.
        let (point, hwnd) = state::with(|s| (s.focus_point, s.focus_hwnd));
        match &before {
            Some(before) => g::settle_screen(before, point, hwnd, 0.8),
            None => state::sleep(0.8),
        }
        let hwnd = hwnd.map(|h| Value::from(h as i64));
        let (note, shot) = g::take_screenshot(true, None, point, None, hwnd.as_ref())?;
        text = format!("{text}\n{note}");
        image = Some(shot);
    }
    Ok((clip(&text), image))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn binding_errors_read_like_python() {
        let msg = |tool: &str, a: Value| check_args(find(tool).unwrap(), &args(a)).unwrap_err().http_message();
        assert_eq!(
            msg("read_file", json!({"pth": 1})),
            "Invalid arguments: t_read_file() got an unexpected keyword argument 'pth'. Did you mean 'path'?"
        );
        assert_eq!(
            msg("read_file", json!({"path": 1, "zzz": 1})),
            "Invalid arguments: t_read_file() got an unexpected keyword argument 'zzz'"
        );
        assert_eq!(
            msg("read_file", json!({})),
            "Invalid arguments: t_read_file() missing 1 required positional argument: 'path'"
        );
        assert_eq!(
            msg("edit_file", json!({})),
            "Invalid arguments: t_edit_file() missing 3 required positional arguments: 'path', 'old', and 'new'"
        );
        assert_eq!(
            msg("drag", json!({"x1": 1})),
            "Invalid arguments: t_drag() missing 3 required positional arguments: 'y1', 'x2', and 'y2'"
        );
        assert_eq!(
            msg("click_elements", json!({})),
            "Invalid arguments: t_click_elements() missing 2 required positional arguments: 'window_id' and 'names'"
        );
        assert!(check_args(find("clipboard_get").unwrap(), &args(json!({}))).is_ok());
    }

    #[test]
    fn clip_keeps_head_and_tail() {
        let long: String = "あ".repeat(MAX_OUTPUT + 10);
        let c = clip(&long);
        assert!(c.contains("...[10 characters truncated]..."));
        assert_eq!(py::len(&c), MAX_OUTPUT + py::len("\n\n...[10 characters truncated]...\n\n"));
        assert_eq!(clip("short"), "short");
    }

    #[test]
    fn tool_table_matches_server_py() {
        assert_eq!(TOOLS.len(), 40);
        for name in DESKTOP_TOOLS {
            assert!(find(name).is_some(), "{name}");
        }
        assert_eq!(tool_names()[..7], ["cd", "list_dir", "read_file", "read_files", "write_file", "edit_file", "edit_files"]);
        for name in SCREEN_ACTIONS.iter().chain(UI_TOOLS.iter()) {
            assert!(find(name).is_some(), "{name}");
        }
    }
}
