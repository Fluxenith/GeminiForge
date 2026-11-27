//! The MCP confirmation popup on macOS (an AppleScript dialog) and Linux
//! (zenity or kdialog). Same exit codes as the Windows window:
//! 0 = allow once, 2 = allow this tool for 30 minutes, 1 = deny / timeout.

use crate::approve::shown_args;
use crate::lang;
use crate::py;
use std::io::Read;
use std::process::{Command, Stdio};

/// macOS dialogs grow with their text, so the arguments are cut shorter than on Windows.
const MAX_SHOWN: usize = 1500;

fn message(tool: &str, raw_args: &str, timeout: i32) -> String {
    let mut args = shown_args(raw_args);
    if py::len(&args) > MAX_SHOWN {
        args = format!("{}\n…", py::prefix(&args, MAX_SHOWN));
    }
    format!(
        "{}\n\n{}\n\n{}",
        lang::t("dialogAsk", &[("tool", tool)]),
        args,
        lang::t("countdown", &[("n", &timeout.to_string())])
    )
}

/// Run a dialog program and wait at most `timeout` seconds (plus a little
/// for the dialog's own timer). Returns (exit code, stdout) or None when the
/// program could not be started.
fn run_dialog(cmd: &mut Command, timeout: i32) -> Option<(i32, String)> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let deadline = crate::state::now() + timeout.max(1) as f64 + 5.0;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if crate::state::now() < deadline => crate::state::sleep(0.05),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    Some((status.and_then(|s| s.code()).unwrap_or(1), out.trim().to_string()))
}

fn has_program(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

/// Why no popup can be shown on this computer, if that is the case.
pub fn missing() -> Option<String> {
    if cfg!(target_os = "macos") {
        return None;
    }
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Some("no desktop session (DISPLAY / WAYLAND_DISPLAY is not set)".to_string());
    }
    if !has_program("zenity") && !has_program("kdialog") {
        return Some("neither zenity nor kdialog is installed".to_string());
    }
    None
}

/// Show the popup and return its exit code.
pub fn show(tool: &str, raw_args: &str, timeout: i32) -> i32 {
    let (title, text) = (lang::t("dialogTitle", &[]), message(tool, raw_args, timeout));
    let (allow, timed, deny) = (lang::t("allow", &[]), lang::t("allowTimed", &[]), lang::t("deny", &[]));
    let pick = |answer: &str| -> i32 {
        if answer == allow {
            0
        } else if answer == timed {
            2
        } else {
            1
        }
    };
    if cfg!(target_os = "macos") {
        // The texts go in as arguments, so nothing in them is read as AppleScript.
        let script = [
            "on run argv",
            "activate",
            "set r to display dialog (item 2 of argv) with title (item 1 of argv) buttons {item 5 of argv, item 4 of argv, item 3 of argv} cancel button 1 giving up after ((item 6 of argv) as integer) with icon caution",
            "if gave up of r then return \"\"",
            "return button returned of r",
            "end run",
        ];
        let mut cmd = Command::new("osascript");
        for line in script {
            cmd.args(["-e", line]);
        }
        let secs = timeout.to_string();
        cmd.args([&title, &text, &allow, &timed, &deny, &secs]);
        return match run_dialog(&mut cmd, timeout) {
            Some((0, answer)) => pick(&answer),
            _ => 1,
        };
    }
    if missing().is_some() {
        return 1;
    }
    if has_program("zenity") {
        let mut cmd = Command::new("zenity");
        cmd.args(["--question", "--no-markup", "--width=560"])
            .arg(format!("--title={title}"))
            .arg(format!("--text={text}"))
            .arg(format!("--ok-label={allow}"))
            .arg(format!("--cancel-label={deny}"))
            .arg(format!("--extra-button={timed}"))
            .arg(format!("--timeout={timeout}"));
        return match run_dialog(&mut cmd, timeout) {
            Some((0, _)) => 0,
            // the extra button exits like Cancel but prints its label
            Some((1, answer)) => pick(&answer),
            _ => 1,
        };
    }
    let mut cmd = Command::new("kdialog");
    cmd.args(["--title", &title, "--yesnocancel", &text])
        .args(["--yes-label", &allow, "--no-label", &timed, "--cancel-label", &deny]);
    match run_dialog(&mut cmd, timeout) {
        Some((0, _)) => 0,
        Some((1, _)) => 2,
        _ => 1,
    }
}
