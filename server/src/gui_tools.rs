//! Screen and input tools: screenshot, zoom, click, double_click, right_click,
//! move, drag, type, key, clipboard_get, clipboard_set, list_windows, focus_window.

use crate::ffi;
use crate::image::{self, Img, Labels};
use crate::input;
use crate::py::{self, Error, Res};
use crate::state::{self, sleep, CAPTURE_TTL, SHOT_MAX_WIDTH};
use crate::tools::{Args, ToolOut};
use crate::win;
use serde_json::{json, Value};

// ---------------------------------------------------------------- helpers

/// Screenshot pixels -> real screen pixels.
pub fn to_real(x: &Value, y: &Value) -> Res<(i32, i32)> {
    let (s, (ox, oy)) = state::with(|st| (st.shot_scale, st.shot_origin));
    let (fx, fy) = (py::float(x)?, py::float(y)?);
    Ok((py::round(ox as f64 + fx * s) as i32, py::round(oy as f64 + fy * s) as i32))
}

/// Real screen pixels -> last screenshot pixels, or None if off that screenshot.
pub fn to_shot(rx: i32, ry: i32) -> Option<(i64, i64)> {
    let (size, s, (ox, oy)) = state::with(|st| (st.shot_size, st.shot_scale, st.shot_origin));
    let (w, h) = size?;
    let (x, y) = ((rx - ox) as f64 / s, (ry - oy) as f64 / s);
    (0.0 <= x && x < w as f64 && 0.0 <= y && y < h as f64).then(|| (py::round(x), py::round(y)))
}

pub fn new_capture_id() -> String {
    format!("cap_{}", ffi::token_hex(4))
}

/// Fail-closed check that a pinned capture_id matches the current snapshot and is fresh.
/// `expected` / `captured_at` of None fall back to the last screenshot.
pub fn validate_capture_id(capture_id: Option<&Value>, expected: Option<&str>, captured_at: Option<f64>) -> Res<()> {
    let Some(capture_id) = capture_id.filter(|v| !v.is_null()) else { return Ok(()) };
    let (shot_id, shot_time) = state::with(|s| (s.shot_capture_id.clone(), s.shot_time));
    let exp = expected.map(|s| s.to_string()).or(shot_id).unwrap_or_default();
    let ts = captured_at.unwrap_or(shot_time);
    if exp.is_empty() || py::py_str(capture_id) != exp {
        return Err(Error::tool("capture_expired: snapshot id does not match the latest observation; take a fresh screenshot or ocr_window."));
    }
    if state::now() - ts > CAPTURE_TTL {
        return Err(Error::tool(format!(
            "capture_expired: snapshot is older than {}s; take a fresh screenshot or ocr_window.",
            CAPTURE_TTL as i64
        )));
    }
    Ok(())
}

/// `_with_restore_focus`: run `f`, then put the user's window and cursor back.
pub fn with_restore_focus<T>(restore_focus: bool, target_hwnd: Option<isize>, f: impl FnOnce() -> Res<T>) -> Res<T> {
    if !restore_focus {
        return f();
    }
    let (prev_hwnd, prev_pos) = win::save_foreground_state();
    let result = f();
    sleep(0.05);
    win::restore_foreground_state(prev_hwnd, prev_pos, target_hwnd);
    result
}

/// The purple coordinate grid and the red circle at the mouse. Returns (cx, cy, inside).
fn draw_grid_and_cursor(img: &mut Img, ox: i32, oy: i32, scale: f64, grid: bool) -> (f64, f64, bool) {
    let (mx, my) = input::position();
    let (cx, cy) = ((mx - ox) as f64 / scale, (my - oy) as f64 / scale);
    let (w, h) = (img.w as i64, img.h as i64);
    if grid {
        let step = 100usize;
        for gx in (0..w).step_by(step) {
            img.fill_rect(gx, 0, gx, h, (255, 0, 255, 60));
        }
        for gy in (0..h).step_by(step) {
            img.fill_rect(0, gy, w, gy, (255, 0, 255, 60));
        }
        let labels = Labels::new();
        for gx in (0..w).step_by(step * 2) {
            for gy in (0..h).step_by(step * 2) {
                let label = format!("{gx},{gy}");
                img.fill_rect(gx + 1, gy + 1, gx + 7 * label.len() as i64, gy + 13, (0, 0, 0, 110));
                labels.draw(img, gx + 2, gy + 1, &label, (255, 255, 0, 255));
            }
        }
    }
    let inside = 0.0 <= cx && cx < w as f64 && 0.0 <= cy && cy < h as f64;
    if inside {
        img.ring(cx, cy, 6.0, 2.0, (255, 0, 0, 255));
    }
    (cx, cy, inside)
}

/// Scale an image down to SHOT_MAX_WIDTH; returns (image, real pixels per image pixel).
pub fn fit_width(img: Img) -> (Img, f64) {
    if img.w > SHOT_MAX_WIDTH {
        let scale = img.w as f64 / SHOT_MAX_WIDTH as f64;
        let h = py::round(img.h as f64 / scale).max(0) as u32;
        (image::resize_lanczos(&img, SHOT_MAX_WIDTH, h), scale)
    } else {
        (img, 1.0)
    }
}

fn remember_shot(scale: f64, origin: (i32, i32), size: (u32, u32), hwnd: Option<isize>, cid: &str) {
    state::with(|s| {
        s.shot_scale = scale;
        s.shot_origin = origin;
        s.shot_size = Some(size);
        s.shot_hwnd = hwnd;
        s.shot_capture_id = Some(cid.to_string());
        s.shot_time = state::now();
    });
}

/// Capture a single window (if window/window_id is given) or one monitor.
/// Returns (note, base64 JPEG).
pub fn take_screenshot(
    grid: bool, monitor: Option<&Value>, point: Option<(i32, i32)>, window: Option<&Value>, window_id: Option<&Value>,
) -> Res<(String, String)> {
    let window = window.filter(|v| !v.is_null());
    let window_id = window_id.filter(|v| !v.is_null());
    if window.is_some() || window_id.is_some() {
        let w = win::find_window(window, window_id)?;
        if w.min || w.hidden {
            win::activate(w.hwnd, w.hidden)?;
        }
        let hwnd = w.hwnd;
        let (img, (ox, oy, _, _), method) = win::capture_window_image(hwnd)?;
        let (mut img, scale) = fit_width(img);
        let cid = new_capture_id();
        remember_shot(scale, (ox, oy), (img.w, img.h), Some(hwnd), &cid);
        let (cx, cy, inside) = draw_grid_and_cursor(&mut img, ox, oy, scale, grid);
        let mouse = if inside {
            format!("red circle = mouse at {},{}", py::round(cx), py::round(cy))
        } else {
            "mouse is outside this window".to_string()
        };
        let note = format!(
            "Window \"{}\" screenshot {}x{} (window_id={hwnd}, capture_id={cid}, method={method}; coordinates are pixels of this window image; purple grid lines every 100px, labeled every 200px; {mouse}).",
            py::prefix(&w.title, 60),
            img.w,
            img.h
        );
        return Ok((note, image::jpeg_base64(&img, 84)?));
    }

    let mons = win::list_monitors();
    if mons.is_empty() {
        return Err(Error::exc("IndexError", "list index out of range"));
    }
    let (mx, my) = input::position();
    let (px, py_) = point.unwrap_or((mx, my));
    let idx = match monitor.filter(|m| py::truthy(m)) {
        Some(m) => py::int(m)?.clamp(1, mons.len() as i64) as usize,
        None => mons
            .iter()
            .position(|m| m.left <= px && px < m.left + m.width && m.top <= py_ && py_ < m.top + m.height)
            .map(|i| i + 1)
            .unwrap_or(1),
    };
    let mon = mons[idx - 1];
    let img = win::grab_screen(mon.left, mon.top, mon.width, mon.height)?;
    let (mut img, scale) = fit_width(img);
    let cid = new_capture_id();
    remember_shot(scale, (mon.left, mon.top), (img.w, img.h), None, &cid);
    let (cx, cy, inside) = draw_grid_and_cursor(&mut img, mon.left, mon.top, scale, grid);
    let others: Vec<String> = mons
        .iter()
        .enumerate()
        .map(|(i, m)| format!("{}={}x{}{}", i + 1, m.width, m.height, if i == 0 { " (primary)" } else { "" }))
        .collect();
    let mouse = if inside {
        format!("red circle = mouse at {},{}", py::round(cx), py::round(cy))
    } else {
        "mouse is not on this monitor".to_string()
    };
    let note = format!(
        "Monitor {idx} screenshot {}x{} (capture_id={cid}; coordinates are pixels of this image; purple grid lines every 100px, labeled every 200px; {mouse}). Monitors: {}. Switch with screenshot {{monitor: N}}.",
        img.w,
        img.h,
        others.join(", ")
    );
    Ok((note, image::jpeg_base64(&img, 82)?))
}

/// A cheap fingerprint of every monitor's picture (fast waits only).
pub struct ScreenMarks(Vec<(win::Monitor, u64)>);

fn monitor_mark(m: &win::Monitor) -> Option<u64> {
    let img = win::grab_screen(m.left, m.top, m.width, m.height).ok()?;
    // FNV-1a over every third pixel of every third row
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let row = img.w as usize * 3;
    for y in (0..img.h as usize).step_by(3) {
        for px in img.rgb[y * row..(y + 1) * row].chunks_exact(3).step_by(3) {
            for b in px {
                h = (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    Some(h)
}

/// What the monitors show right now, taken before a screen action.
pub fn screen_marks() -> ScreenMarks {
    ScreenMarks(win::list_monitors().into_iter().filter_map(|m| monitor_mark(&m).map(|h| (m, h))).collect())
}

/// Fast wait before the screenshot that follows a screen action: go on once
/// the monitor has changed since `before` and then stayed the same for 0.2 s.
/// A screen that never changes, or never stops changing (a video), gets the
/// full `max` seconds, as with the fixed wait.
pub fn settle_screen(before: &ScreenMarks, point: Option<(i32, i32)>, hwnd: Option<isize>, max: f64) {
    const STEADY: f64 = 0.2;
    let (px, py_) = point.unwrap_or_else(input::position);
    let mons = win::list_monitors();
    let mon = mons
        .iter()
        .find(|m| m.left <= px && px < m.left + m.width && m.top <= py_ && py_ < m.top + m.height)
        .or(mons.first())
        .copied();
    let known = mon.and_then(|m| before.0.iter().find(|(b, _)| *b == m).map(|(_, h)| (m, *h)));
    // a window-only screenshot (background clicks) keeps the fixed wait
    let (Some((mon, before_mark)), None) = (known, hwnd) else {
        sleep(max);
        return;
    };
    let deadline = state::now() + max;
    let (mut last, mut last_change, mut changed) = (before_mark, state::now(), false);
    loop {
        let Some(mark) = monitor_mark(&mon) else {
            sleep((deadline - state::now()).max(0.0));
            return;
        };
        let now = state::now();
        if mark != last {
            last = mark;
            last_change = now;
        }
        changed |= mark != before_mark;
        if (changed && now - last_change >= STEADY) || now >= deadline {
            return;
        }
        sleep(0.05f64.min(deadline - now));
    }
}

/// `type_text`: plain ASCII is typed key by key, anything else is pasted.
pub fn type_text(text: &str) -> Res<()> {
    if text.is_ascii() && !text.contains('\n') {
        return input::write(text, 0.01);
    }
    let old = win::clipboard_get().ok();
    win::clipboard_set(text)?;
    sleep(0.1);
    input::hotkey(&["ctrl", "v"])?;
    sleep(0.3);
    if let Some(old) = old {
        let _ = win::clipboard_set(&old);
    }
    Ok(())
}

pub fn parse_keys(keys: &Value) -> Vec<String> {
    py::py_str(keys)
        .split('+')
        .map(|k| py::strip(k).to_lowercase())
        .filter(|k| !k.is_empty())
        .map(|k| {
            match k.as_str() {
                "control" => "ctrl",
                "windows" | "cmd" => "win",
                "return" => "enter",
                "escape" => "esc",
                "del" => "delete",
                "pgup" => "pageup",
                "pgdn" => "pagedown",
                other => other,
            }
            .to_string()
        })
        .collect()
}

/// `int(window_id) if window_id is not None else None`
pub fn opt_hwnd(a: &Args) -> Res<Option<isize>> {
    a.opt("window_id").map(|v| py::int(v).map(|i| i as isize)).transpose()
}

fn activate_target(hwnd: isize) -> Res<win::Win> {
    let w = win::find_window_by_id(hwnd)?;
    win::activate(w.hwnd, w.hidden)?;
    Ok(w)
}

fn set_focus_point(p: (i32, i32)) {
    state::with(|s| s.focus_point = Some(p));
}

fn restored_suffix(restore_focus: bool) -> &'static str {
    if restore_focus {
        " (restored previous window)"
    } else {
        ""
    }
}

/// A click that leaves the user's foreground window alone; the screenshot after it is of that window.
pub fn click_without_focus(
    hwnd: isize, rx: i32, ry: i32, button: &str, clicks: &Value, background: bool, what: &str,
) -> Res<String> {
    let n = py::int(clicks)?;
    let clicked = if background {
        win::post_background_click(hwnd, rx, ry, button, n)
    } else {
        win::cursor_jump_click(hwnd, rx, ry, button, n)
    };
    let (target, (cx, cy)) = clicked.map_err(|e| match e {
        Error::Exc(kind, msg) if kind == "RuntimeError" => Error::Tool(msg),
        other => other,
    })?;
    state::with(|s| {
        s.focus_point = Some((rx, ry));
        s.focus_hwnd = Some(hwnd);
    });
    let (how, mouse) = if background {
        ("Background-clicked", "mouse not moved")
    } else {
        ("Cursor-jump-clicked", "window not activated, cursor put back")
    };
    Ok(format!("{how} {what} in window_id={hwnd} (child={target}, client={cx},{cy}; {mouse})"))
}

pub fn valid_button(button: &Value) -> Res<String> {
    match button.as_str() {
        Some(b) if matches!(b, "left" | "right" | "middle") => Ok(b.to_string()),
        _ => Err(Error::tool(format!("Invalid mouse button: {}", py::py_str(button)))),
    }
}

// ---------------------------------------------------------------- tools

pub fn t_screenshot(a: &Args) -> Res<ToolOut> {
    let (note, image) =
        take_screenshot(a.truthy("grid", true), a.get("monitor"), None, a.get("window"), a.get("window_id"))?;
    Ok(ToolOut { text: note, image: Some(image) })
}

fn click_impl(a: &Args, x: &Value, y: &Value, button: Value, clicks: Value) -> Res<ToolOut> {
    let button = valid_button(&button)?;
    validate_capture_id(a.get("capture_id"), None, None)?;
    let (rx, ry) = to_real(x, y)?;
    let (background, cursor_jump) = (a.truthy("background", false), a.truthy("cursor_jump", false));
    let restore_focus = a.truthy("restore_focus", false);
    let what = format!("{button} button {}x at ({},{})", py::py_str(&clicks), py::py_str(x), py::py_str(y));
    if background || cursor_jump {
        let hwnd = match opt_hwnd(a)? {
            Some(h) => Some(h),
            None => state::with(|s| s.shot_hwnd),
        };
        let Some(hwnd) = hwnd.filter(|h| *h != 0) else {
            let mode = if background { "background" } else { "cursor_jump" };
            return Err(Error::tool(format!("{mode} click requires window_id (or a prior window screenshot).")));
        };
        win::find_window_by_id(hwnd)?;
        return Ok(click_without_focus(hwnd, rx, ry, &button, &clicks, background, &what)?.into());
    }
    let target = opt_hwnd(a)?;
    with_restore_focus(restore_focus, target, || {
        if let Some(hwnd) = target {
            activate_target(hwnd)?;
        }
        set_focus_point((rx, ry));
        input::click(rx, ry, py::int(&clicks)?, 0.08, &button)
    })?;
    Ok(format!("Clicked {what}{}", restored_suffix(restore_focus)).into())
}

pub fn t_click(a: &Args) -> Res<ToolOut> {
    click_impl(a, a.req("x"), a.req("y"), a.or("button", json!("left")), a.or("clicks", json!(1)))
}

pub fn t_double_click(a: &Args) -> Res<ToolOut> {
    click_impl(a, a.req("x"), a.req("y"), json!("left"), json!(2))
}

pub fn t_right_click(a: &Args) -> Res<ToolOut> {
    click_impl(a, a.req("x"), a.req("y"), json!("right"), json!(1))
}

pub fn t_move(a: &Args) -> Res<ToolOut> {
    let (x, y) = (a.req("x"), a.req("y"));
    let (rx, ry) = to_real(x, y)?;
    input::move_to(rx, ry, 0.2)?;
    Ok(format!("Mouse moved to ({},{})", py::py_str(x), py::py_str(y)).into())
}

pub fn t_drag(a: &Args) -> Res<ToolOut> {
    let (x1, y1, x2, y2) = (a.req("x1"), a.req("y1"), a.req("x2"), a.req("y2"));
    let (ax, ay) = to_real(x1, y1)?;
    input::move_to(ax, ay, 0.0)?;
    let (bx, by) = to_real(x2, y2)?;
    input::drag_to(bx, by, 0.5)?;
    Ok(format!(
        "Dragged from ({},{}) to ({},{})",
        py::py_str(x1),
        py::py_str(y1),
        py::py_str(x2),
        py::py_str(y2)
    )
    .into())
}

pub fn t_type(a: &Args) -> Res<ToolOut> {
    let text = py::py_str(a.req("text"));
    let enter = a.truthy("enter", false);
    let restore_focus = a.truthy("restore_focus", false);
    let target = opt_hwnd(a)?;
    with_restore_focus(restore_focus, target, || {
        if let Some(hwnd) = target {
            let w = activate_target(hwnd)?;
            set_focus_point(win::window_center(w.hwnd));
        }
        type_text(&text)?;
        if enter {
            input::press("enter")?;
        }
        Ok(())
    })?;
    Ok(format!(
        "Typed {} characters{}{}",
        py::len(&text),
        if enter { " and pressed Enter" } else { "" },
        restored_suffix(restore_focus)
    )
    .into())
}

pub fn t_key(a: &Args) -> Res<ToolOut> {
    let ks = parse_keys(a.req("keys"));
    if ks.is_empty() {
        return Err(Error::tool("Provide at least one key."));
    }
    let restore_focus = a.truthy("restore_focus", false);
    let target = opt_hwnd(a)?;
    with_restore_focus(restore_focus, target, || {
        if let Some(hwnd) = target {
            let w = activate_target(hwnd)?;
            set_focus_point(win::window_center(w.hwnd));
        }
        if ks.len() == 1 {
            input::press(&ks[0])
        } else {
            let refs: Vec<&str> = ks.iter().map(|k| k.as_str()).collect();
            input::hotkey(&refs)
        }
    })?;
    Ok(format!("Pressed {}{}", ks.join("+"), restored_suffix(restore_focus)).into())
}

pub fn t_clipboard_get(_a: &Args) -> Res<ToolOut> {
    Ok(win::clipboard_get()?.into())
}

pub fn t_clipboard_set(a: &Args) -> Res<ToolOut> {
    win::clipboard_set(&py::py_str(a.req("text")))?;
    Ok("Copied to clipboard".into())
}

pub fn t_list_windows(_a: &Args) -> Res<ToolOut> {
    let fg = win::foreground();
    let lines: Vec<String> = win::top_windows()
        .iter()
        .take(60)
        .map(|w| {
            let (x, y, wd, ht) = w.rect;
            let state = if w.hidden {
                "(hidden; bring it back with focus_window, or use ui_elements/click_element directly)".to_string()
            } else if w.min {
                "(minimized)".to_string()
            } else {
                format!("at ({x},{y}) size {wd}x{ht}")
            };
            let mark = if w.hwnd == fg { " <- active" } else { "" };
            format!("- window_id={} \"{}\" {state}{mark}", w.hwnd, py::prefix(&w.title, 80))
        })
        .collect();
    Ok(format!("Visible windows:\n{}", lines.join("\n")).into())
}

pub fn t_focus_window(a: &Args) -> Res<ToolOut> {
    let w = win::find_window(a.get("title"), a.get("window_id"))?;
    win::activate(w.hwnd, w.hidden)?;
    set_focus_point(win::window_center(w.hwnd));
    Ok(format!("Switched to window \"{}\"", py::prefix(&w.title, 80)).into())
}

/// Close-up of part of the last screenshot; does not change click coordinates.
pub fn t_zoom(a: &Args) -> Res<ToolOut> {
    let (x1, y1, x2, y2) = (a.req("x1"), a.req("y1"), a.req("x2"), a.req("y2"));
    if state::with(|s| s.shot_size).is_none() {
        return Err(Error::tool("Run screenshot first"));
    }
    let ((rx1, ry1), (rx2, ry2)) = (to_real(x1, y1)?, to_real(x2, y2)?);
    let (left, top) = (rx1.min(rx2), ry1.min(ry2));
    let (w, h) = ((rx2 - rx1).abs(), (ry2 - ry1).abs());
    if w < 10 || h < 10 {
        return Err(Error::tool("Area too small"));
    }
    let mut img = win::grab_screen(left, top, w, h)?;
    let factor = 3.0f64.min(SHOT_MAX_WIDTH as f64 / img.w as f64).min(1200.0 / img.h as f64);
    if factor > 1.0 {
        let (nw, nh) = (py::round(img.w as f64 * factor) as u32, py::round(img.h as f64 * factor) as u32);
        img = image::resize_lanczos(&img, nw, nh);
    }
    let note = format!(
        "Zoomed view of ({},{})-({},{}) from the last screenshot. Note: clicks still use the coordinates of the full screenshot.",
        py::py_str(x1),
        py::py_str(y1),
        py::py_str(x2),
        py::py_str(y2)
    );
    Ok(ToolOut { text: note, image: Some(image::jpeg_base64(&img, 88)?) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_split_lowered_and_aliased() {
        assert_eq!(parse_keys(&json!("Control + Shift+ESCAPE")), vec!["ctrl", "shift", "esc"]);
        assert_eq!(parse_keys(&json!("cmd+pgdn")), vec!["win", "pagedown"]);
        assert!(parse_keys(&json!(" + ")).is_empty());
        assert_eq!(parse_keys(&json!("a")), vec!["a"]);
    }

    #[test]
    fn invalid_button_is_a_tool_error() {
        assert_eq!(valid_button(&json!("sideways")).unwrap_err(), Error::tool("Invalid mouse button: sideways"));
        assert_eq!(valid_button(&json!(3)).unwrap_err(), Error::tool("Invalid mouse button: 3"));
        assert_eq!(valid_button(&json!("right")).unwrap(), "right");
    }
}
