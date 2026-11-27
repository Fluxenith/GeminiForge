//! Desktop tools on macOS and Linux: screenshots and the clipboard work
//! through the system's own programs; the mouse, keyboard and window tools
//! are not here yet.

use super::ffi;
use crate::image::{self, Img};
use crate::py::{self, Error, Res};
use crate::state::{self, SHOT_MAX_WIDTH};
use crate::tools::{Args, ToolOut};
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

not_here!(
    t_click, t_double_click, t_right_click, t_move, t_drag, t_type, t_key, t_list_windows, t_focus_window, t_zoom,
);

/// A capture program gets this long before it is given up on.
const CAPTURE_SECONDS: u64 = 20;

/// screenshot: the whole screen (on macOS one display with `monitor`), sized
/// down to SHOT_MAX_WIDTH like on Windows. There is no grid: nothing can be
/// clicked here yet.
pub fn t_screenshot(a: &Args) -> Res<ToolOut> {
    if a.opt("window").is_some() || a.opt("window_id").is_some() {
        return Err(Error::tool(format!(
            "Window screenshots are not available on {} yet. Leave out window and window_id to capture the screen.",
            state::OS_NAME
        )));
    }
    let display = match a.opt("monitor").filter(|m| py::truthy(m)) {
        Some(m) => Some(py::int(m)?.max(1)),
        None => None,
    };
    let img = decode_png(&capture_png(display)?)?;
    let (w, h) = (img.w, img.h);
    let img = fit_width(img);
    let what = match display {
        Some(n) if cfg!(target_os = "macos") => format!("display {n}"),
        _ => "screen".to_string(),
    };
    let mut note = format!(
        "Screenshot of the {what}, {w}x{h} pixels, shown at {}x{}. It is for looking only: clicking and typing are not available on {} yet.",
        img.w,
        img.h,
        state::OS_NAME
    );
    if display.is_some() && !cfg!(target_os = "macos") {
        note.push_str(" (monitor is ignored on Linux: this is the whole desktop.)");
    }
    Ok(ToolOut { text: note, image: Some(image::jpeg_base64(&img, 82)?) })
}

/// Shrink to SHOT_MAX_WIDTH wide, keeping the shape (Retina and 4K screens are much wider).
fn fit_width(img: Img) -> Img {
    if img.w <= SHOT_MAX_WIDTH {
        return img;
    }
    let h = (img.h as f64 * SHOT_MAX_WIDTH as f64 / img.w as f64).round().max(1.0) as u32;
    image::resize_lanczos(&img, SHOT_MAX_WIDTH, h)
}

/// The capture programs to try, in order, each told to write `file` as PNG.
fn capture_commands(file: &str, display: Option<i64>) -> Vec<Vec<String>> {
    let cmd = |parts: &[&str]| parts.iter().map(|p| p.to_string()).collect::<Vec<String>>();
    if cfg!(target_os = "macos") {
        let mut c = cmd(&["screencapture", "-x", "-t", "png"]);
        if let Some(n) = display {
            c.extend(["-D".to_string(), n.to_string()]);
        }
        c.push(file.to_string());
        return vec![c];
    }
    // Wayland desktops first (grim: Sway, Hyprland; gnome-screenshot: GNOME;
    // spectacle: KDE), then X11 (scrot, ImageMagick's import)
    vec![
        cmd(&["grim", file]),
        cmd(&["gnome-screenshot", "-f", file]),
        cmd(&["spectacle", "-b", "-n", "-f", "-o", file]),
        cmd(&["scrot", file]),
        cmd(&["import", "-window", "root", file]),
    ]
}

/// Run the capture programs until one writes the picture; returns the PNG bytes.
fn capture_png(display: Option<i64>) -> Res<Vec<u8>> {
    let file = std::env::temp_dir().join(format!("geminiforge_shot_{}.png", ffi::token_hex(4)));
    let path = file.to_string_lossy().into_owned();
    let (mut found, mut last) = (false, String::new());
    for cmd in capture_commands(&path, display) {
        let _ = std::fs::remove_file(&file);
        let child =
            Command::new(&cmd[0]).args(&cmd[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn();
        let child = match child {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                last = format!("{}: {e}", cmd[0]);
                continue;
            }
        };
        found = true;
        let pid = child.id();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });
        match rx.recv_timeout(Duration::from_secs(CAPTURE_SECONDS)) {
            Ok(Ok(out)) => match std::fs::read(&file) {
                Ok(data) if out.status.success() && !data.is_empty() => {
                    let _ = std::fs::remove_file(&file);
                    return Ok(data);
                }
                _ => last = format!("{}: {}", cmd[0], py::strip(&String::from_utf8_lossy(&out.stderr))),
            },
            Ok(Err(e)) => last = format!("{}: {e}", cmd[0]),
            Err(_) => {
                ffi::kill_tree(pid);
                last = format!("{} did not finish in {CAPTURE_SECONDS} s", cmd[0]);
            }
        }
    }
    let _ = std::fs::remove_file(&file);
    Err(Error::tool(if cfg!(target_os = "macos") {
        format!(
            "The screenshot failed ({last}). macOS needs Screen Recording permission for the app that started GeminiForge (Terminal, for example): System Settings > Privacy & Security > Screen Recording."
        )
    } else if !found {
        "No screenshot program found. Install one: grim (Sway, Hyprland), gnome-screenshot (GNOME), spectacle (KDE), or scrot or ImageMagick (X11); for example: sudo apt install gnome-screenshot".to_string()
    } else {
        format!("The screenshot failed ({last}).")
    }))
}

/// A PNG as an RGB image (palette, 16-bit, grey and alpha are converted).
fn decode_png(data: &[u8]) -> Res<Img> {
    let bad = |e: png::DecodingError| Error::tool(format!("The screenshot could not be read: {e}"));
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(bad)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(bad)?;
    let px = &buf[..info.buffer_size()];
    let rgb: Vec<u8> = match info.color_type {
        png::ColorType::Rgb => px.to_vec(),
        png::ColorType::Rgba => px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0]]).collect(),
        png::ColorType::Indexed => return Err(Error::tool("The screenshot could not be read: unexpected palette image")),
    };
    Ok(Img { w: info.width, h: info.height, rgb })
}

/// Only Windows takes a screenshot after screen actions, so `tools::call_tool` never asks for these.
pub struct ScreenMarks;

pub fn screen_marks() -> ScreenMarks {
    ScreenMarks
}

pub fn settle_screen(_before: &ScreenMarks, _point: Option<(i32, i32)>, _hwnd: Option<isize>, _max: f64) {}

pub fn take_screenshot(
    _grid: bool, _monitor: Option<&Value>, _point: Option<(i32, i32)>, _window: Option<&Value>,
    _window_id: Option<&Value>,
) -> Res<(String, String)> {
    Err(crate::tools::unavailable("screenshot"))
}

/// (read command, write command)
type Pair = (&'static [&'static str], &'static [&'static str]);
const PBCOPY: Pair = (&["pbpaste"], &["pbcopy"]);
const WL_COPY: Pair = (&["wl-paste", "--no-newline"], &["wl-copy"]);
const XCLIP: Pair = (&["xclip", "-selection", "clipboard", "-o"], &["xclip", "-selection", "clipboard", "-i"]);
const XSEL: Pair = (&["xsel", "--clipboard", "--output"], &["xsel", "--clipboard", "--input"]);

/// The clipboard commands to try, in order.
fn clipboard_commands() -> Vec<Pair> {
    if cfg!(target_os = "macos") {
        return vec![PBCOPY];
    }
    let mut cmds = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        cmds.push(WL_COPY);
    }
    cmds.push(XCLIP);
    cmds.push(XSEL);
    cmds
}

fn no_clipboard() -> Error {
    if cfg!(target_os = "macos") {
        Error::tool("The clipboard commands pbcopy / pbpaste are not available.")
    } else {
        Error::tool(
            "No clipboard command found. Install wl-clipboard (Wayland) or xclip (X11), for example: sudo apt install wl-clipboard xclip",
        )
    }
}

pub fn t_clipboard_get(_a: &Args) -> Res<ToolOut> {
    for (read, _) in clipboard_commands() {
        let out = Command::new(read[0]).args(&read[1..]).stdin(Stdio::null()).stderr(Stdio::null()).output();
        match out {
            Ok(out) if out.status.success() => return Ok(String::from_utf8_lossy(&out.stdout).into_owned().into()),
            // an empty clipboard makes wl-paste / xclip fail
            Ok(_) => return Ok(String::new().into()),
            Err(_) => continue,
        }
    }
    Err(no_clipboard())
}

pub fn t_clipboard_set(a: &Args) -> Res<ToolOut> {
    let text = py::py_str(a.req("text"));
    for (_, write) in clipboard_commands() {
        let child = Command::new(write[0])
            .args(&write[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { continue };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        // wl-copy and xclip stay running in the background to serve the
        // clipboard; they fork first, so the command we started returns.
        return match child.wait() {
            Ok(status) if status.success() => Ok("Copied to clipboard".into()),
            _ => Err(Error::tool(format!("{} could not set the clipboard", write[0]))),
        };
    }
    Err(no_clipboard())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32, color: png::ColorType, px: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(px).unwrap();
        out
    }

    #[test]
    fn png_screens_become_rgb() {
        let rgba = png_bytes(2, 1, png::ColorType::Rgba, &[10, 20, 30, 255, 40, 50, 60, 0]);
        assert_eq!(decode_png(&rgba).unwrap(), Img { w: 2, h: 1, rgb: vec![10, 20, 30, 40, 50, 60] });
        let grey = png_bytes(1, 2, png::ColorType::Grayscale, &[7, 9]);
        assert_eq!(decode_png(&grey).unwrap().rgb, vec![7, 7, 7, 9, 9, 9]);
        assert!(decode_png(b"not a png").is_err());
    }

    #[test]
    fn wide_screens_are_sized_down() {
        let big = fit_width(Img::new(2 * SHOT_MAX_WIDTH, 1000));
        assert_eq!((big.w, big.h), (SHOT_MAX_WIDTH, 500));
        let small = fit_width(Img::new(800, 600));
        assert_eq!((small.w, small.h), (800, 600));
    }

    #[test]
    fn capture_programs_write_the_given_file() {
        for cmd in capture_commands("/tmp/x.png", Some(2)) {
            assert_eq!(cmd.last().map(|s| s.as_str()), Some("/tmp/x.png"), "{cmd:?}");
        }
        if cfg!(target_os = "macos") {
            assert!(capture_commands("/tmp/x.png", Some(2))[0].windows(2).any(|w| w == ["-D", "2"]));
        }
    }
}
