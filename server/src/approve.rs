//! Desktop confirmation popup for MCP tool calls (port of server/approve_dialog.py).
//!
//! Usage: `geminiforge-server approve <tool> <args-json | -> <timeout-seconds>`
//! Exit code: 0 = allow once, 2 = allow this tool for 30 minutes, 1 = deny / timeout.
//!
//! The popup itself is a Win32 window on Windows (approve_win32.rs) and a
//! system dialog on macOS and Linux (unix/popup.rs).

use crate::py;
use serde_json::Value;
use std::io::Read;

#[cfg(windows)]
#[path = "approve_win32.rs"]
mod popup;
#[cfg(not(windows))]
use crate::unix::popup;

pub use popup::show;

/// The arguments as the popup shows them: pretty JSON, at most 2500 characters.
pub fn shown_args(raw: &str) -> String {
    let mut text = match serde_json::from_str::<Value>(raw) {
        Ok(v) => py::json_pretty(&v, 2),
        Err(_) => raw.to_string(),
    };
    if py::len(&text) > 2500 {
        text = format!("{}\n…", py::prefix(&text, 2500));
    }
    text
}

/// `geminiforge-server approve ...`: returns the process exit code.
pub fn main(argv: &[String]) -> i32 {
    let tool = argv.first().map(|s| s.as_str()).unwrap_or("?");
    let raw = match argv.get(1).map(|s| s.as_str()) {
        Some("-") => {
            let mut bytes = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        }
        Some(text) => text.to_string(),
        None => String::new(),
    };
    let timeout = argv.get(2).and_then(|s| s.trim().parse().ok()).unwrap_or(120);
    show(tool, &raw, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_shown_as_pretty_json_and_capped() {
        assert_eq!(shown_args("{\"path\":\"x.txt\",\"n\":[1,2]}"), "{\n  \"path\": \"x.txt\",\n  \"n\": [\n    1,\n    2\n  ]\n}");
        assert_eq!(shown_args("not json"), "not json");
        let long = format!("{{\"content\": \"{}\"}}", "a".repeat(4000));
        let shown = shown_args(&long);
        assert_eq!(py::len(&shown), 2502);
        assert!(shown.ends_with("\n…"));
    }
}
