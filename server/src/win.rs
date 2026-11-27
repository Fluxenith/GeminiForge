//! Window, clipboard and capture helpers (the Win32 parts of server.py and win_desktop.py).

use crate::ffi::{self, POINT, RECT};
use crate::image::Img;
use crate::input;
use crate::py::{self, Error, Res};
use crate::state::{fast_wait, now, poll_until, sleep};
use serde_json::Value;
use std::ffi::c_void;

// ---------------------------------------------------------------- top-level windows

#[derive(Debug, Clone, PartialEq)]
pub struct Win {
    pub hwnd: isize,
    pub title: String,
    pub min: bool,
    pub hidden: bool,
    /// left, top, width, height
    pub rect: (i32, i32, i32, i32),
}

/// `GetWindowTextW` (not stripped).
pub fn window_text(hwnd: isize) -> String {
    unsafe {
        let n = ffi::GetWindowTextLengthW(hwnd);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; n as usize + 1];
        let got = ffi::GetWindowTextW(hwnd, buf.as_mut_ptr(), n + 1);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

pub fn window_rect(hwnd: isize) -> Option<RECT> {
    let mut r = RECT::default();
    (unsafe { ffi::GetWindowRect(hwnd, &mut r) } != 0).then_some(r)
}

pub fn foreground() -> isize {
    unsafe { ffi::GetForegroundWindow() }
}

pub fn is_window(hwnd: isize) -> bool {
    unsafe { ffi::IsWindow(hwnd) != 0 }
}

pub fn window_process_id(hwnd: isize) -> u32 {
    let mut pid = 0u32;
    unsafe { ffi::GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

/// `_window_center`
pub fn window_center(hwnd: isize) -> (i32, i32) {
    let r = window_rect(hwnd).unwrap_or_default();
    ((r.left + r.right).div_euclid(2), (r.top + r.bottom).div_euclid(2))
}

unsafe extern "system" fn enum_cb(hwnd: isize, lparam: isize) -> i32 {
    let out = &mut *(lparam as *mut Vec<Win>);
    if ffi::IsWindowVisible(hwnd) == 0 {
        return 1;
    }
    if ffi::GetWindowTextLengthW(hwnd) == 0 {
        return 1;
    }
    let mut cloaked: i32 = 0;
    ffi::DwmGetWindowAttribute(hwnd, 14, &mut cloaked as *mut i32 as *mut c_void, 4); // DWMWA_CLOAKED
    // 2 = hidden by the shell (e.g. a freshly launched Store app such as
    // Calculator, or one on another virtual desktop): keep it, marked hidden.
    if cloaked != 0 && cloaked != 2 {
        return 1;
    }
    let ex = ffi::GetWindowLongW(hwnd, -20); // GWL_EXSTYLE
    if ex & 0x80 != 0 && ex & 0x40000 == 0 {
        return 1; // tool window, not app window
    }
    let title = py::strip(&window_text(hwnd)).to_string();
    if title.is_empty() || title == "Program Manager" {
        return 1;
    }
    let mut r = RECT::default();
    ffi::GetWindowRect(hwnd, &mut r);
    let minimized = ffi::IsIconic(hwnd) != 0;
    if !minimized && (r.right - r.left < 80 || r.bottom - r.top < 40) {
        return 1;
    }
    out.push(Win {
        hwnd,
        title,
        min: minimized,
        hidden: cloaked != 0,
        rect: (r.left, r.top, r.right - r.left, r.bottom - r.top),
    });
    1
}

/// Visible top-level windows with a title, including minimized ones.
pub fn top_windows() -> Vec<Win> {
    let mut out: Vec<Win> = Vec::new();
    unsafe { ffi::EnumWindows(enum_cb, &mut out as *mut Vec<Win> as isize) };
    out
}

/// `_find_window(title, window_id)`; both are the raw tool arguments (None when not given).
pub fn find_window(title: Option<&Value>, window_id: Option<&Value>) -> Res<Win> {
    pick_window(top_windows, title, window_id)
}

/// The choice `_find_window` makes among the open windows.
pub fn pick_window(top_windows: impl Fn() -> Vec<Win>, title: Option<&Value>, window_id: Option<&Value>) -> Res<Win> {
    let title = title.filter(|v| !v.is_null());
    if let Some(id) = window_id.filter(|v| !v.is_null()) {
        let id = py::int(id)? as isize;
        let found = top_windows().into_iter().find(|w| w.hwnd == id);
        let title_changed = |w: &Win| {
            title.filter(|t| py::truthy(t)).map(|t| w.title.to_lowercase() != py::py_str(t).to_lowercase()).unwrap_or(false)
        };
        return match found {
            Some(w) if !title_changed(&w) => Ok(w),
            _ => Err(Error::tool("That window is gone or its title changed; run list_windows / ui_elements again.")),
        };
    }
    let text = title.filter(|t| py::truthy(t)).map(py::py_str).unwrap_or_default();
    if py::strip(&text).is_empty() {
        return Err(Error::tool("Provide a window title or window_id."));
    }
    let t = text.to_lowercase();
    let mut wins: Vec<Win> = top_windows().into_iter().filter(|w| w.title.to_lowercase().contains(&t)).collect();
    if wins.is_empty() {
        return Err(Error::tool(format!(
            "No window title contains \"{text}\". Use list_windows to see open windows."
        )));
    }
    // Best match first: exact title, then title starting with it, then the
    // shortest title. Otherwise "Calculator" could match a browser tab titled
    // "Calculator automation test - Google Chrome" instead of Calculator itself.
    wins.sort_by_key(|w| {
        let lower = w.title.to_lowercase();
        (lower != t, !lower.starts_with(&t), py::len(&w.title), w.hidden)
    });
    Ok(wins.remove(0))
}

pub fn find_window_by_id(hwnd: isize) -> Res<Win> {
    find_window(None, Some(&Value::from(hwnd as i64)))
}

/// `_activate(win)`
pub fn activate(hwnd: isize, hidden: bool) -> Res<()> {
    let restore = unsafe { ffi::IsIconic(hwnd) != 0 } || hidden;
    let fast = fast_wait();
    if fast && !restore && foreground() == hwnd {
        // Already in front: nothing to switch and nothing to wait for. Skipping
        // the Alt tap also keeps the window out of menu mode, where it would
        // swallow the next key.
        return Ok(());
    }
    if restore {
        unsafe { ffi::ShowWindow(hwnd, 9) }; // SW_RESTORE (also brings a hidden Store app back)
    }
    // Windows only lets the foreground process change focus; a tap of Alt lifts that lock.
    input::press("alt")?;
    unsafe { ffi::SetForegroundWindow(hwnd) };
    if fast && !restore {
        // go on as soon as the window is in front, plus a moment for its focus to settle
        poll_until(0.45, 0.015, || foreground() == hwnd);
        sleep(0.05);
    } else {
        // a restored window still has to be drawn: keep the full wait
        sleep(0.5);
    }
    if foreground() != hwnd {
        return Err(Error::tool(
            "Could not bring the target window to the front; stopped to avoid acting on another program.",
        ));
    }
    Ok(())
}

/// `_validate_ui_window(info)`: returns the current (unstripped) title.
pub fn validate_ui_window(hwnd: isize, title: &str, pid: u32) -> Res<String> {
    let current = window_text(hwnd);
    if !is_window(hwnd) || py::strip(&current) != py::strip(title) || window_process_id(hwnd) != pid {
        return Err(Error::tool(
            "The original window closed or changed; run ui_elements again. Old coordinates were not clicked.",
        ));
    }
    Ok(current)
}

// ---------------------------------------------------------------- clipboard (pyperclip)

const CF_UNICODETEXT: u32 = 13;

fn open_clipboard(hwnd: isize) -> Res<()> {
    // Another application may hold the clipboard; try for half a second.
    let deadline = now() + 0.5;
    loop {
        if unsafe { ffi::OpenClipboard(hwnd) } != 0 {
            return Ok(());
        }
        if now() >= deadline {
            return Err(Error::exc("PyperclipWindowsException", "Error calling OpenClipboard"));
        }
        sleep(0.01);
    }
}

/// `pyperclip.paste()`
pub fn clipboard_get() -> Res<String> {
    open_clipboard(0)?;
    let text = unsafe {
        let handle = ffi::GetClipboardData(CF_UNICODETEXT);
        if handle == 0 {
            String::new()
        } else {
            let ptr = ffi::GlobalLock(handle) as *const u16;
            if ptr.is_null() {
                String::new()
            } else {
                let mut n = 0usize;
                while *ptr.add(n) != 0 {
                    n += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, n));
                ffi::GlobalUnlock(handle);
                s
            }
        }
    };
    unsafe { ffi::CloseClipboard() };
    Ok(text)
}

/// `pyperclip.copy(text)`
pub fn clipboard_set(text: &str) -> Res<()> {
    unsafe {
        let hwnd = ffi::CreateWindowExA(0, b"STATIC\0".as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, 0, 0, 0, std::ptr::null_mut());
        let opened = open_clipboard(hwnd);
        if opened.is_ok() {
            ffi::EmptyClipboard();
            if !text.is_empty() {
                // like wcslen: the text ends at the first NUL
                let wide: Vec<u16> = text.encode_utf16().take_while(|c| *c != 0).chain(std::iter::once(0)).collect();
                let handle = ffi::GlobalAlloc(0x0002, wide.len() * 2); // GMEM_MOVEABLE
                if handle != 0 {
                    let dst = ffi::GlobalLock(handle) as *mut u16;
                    if !dst.is_null() {
                        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                        ffi::GlobalUnlock(handle);
                        ffi::SetClipboardData(CF_UNICODETEXT, handle);
                    }
                }
            }
            ffi::CloseClipboard();
        }
        if hwnd != 0 {
            ffi::DestroyWindow(hwnd);
        }
        opened
    }
}

// ---------------------------------------------------------------- screen capture (mss)

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

unsafe extern "system" fn monitor_cb(hmon: isize, _dc: isize, rect: *mut RECT, lparam: isize) -> i32 {
    let out = &mut *(lparam as *mut Vec<Monitor>);
    let mut info = ffi::MONITORINFO { cbSize: std::mem::size_of::<ffi::MONITORINFO>() as u32, ..Default::default() };
    ffi::GetMonitorInfoW(hmon, &mut info);
    let r = *rect;
    out.push(Monitor {
        left: r.left,
        top: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
        primary: info.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
    });
    1
}

/// Physical monitors, primary first.
pub fn list_monitors() -> Vec<Monitor> {
    let mut mons: Vec<Monitor> = Vec::new();
    unsafe { ffi::EnumDisplayMonitors(0, std::ptr::null(), monitor_cb, &mut mons as *mut Vec<Monitor> as isize) };
    mons.sort_by_key(|m| (!m.primary, m.left, m.top));
    mons
}

/// `sct.grab({left, top, width, height})` as an RGB image.
pub fn grab_screen(left: i32, top: i32, width: i32, height: i32) -> Res<Img> {
    if width <= 0 || height <= 0 {
        return Err(Error::exc("ScreenShotError", format!("Region has zero or negative size: {width}x{height}")));
    }
    unsafe {
        let srcdc = ffi::GetWindowDC(0);
        let memdc = ffi::CreateCompatibleDC(srcdc);
        let bmi = ffi::BITMAPINFOHEADER {
            biSize: std::mem::size_of::<ffi::BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = ffi::CreateDIBSection(memdc, &bmi, 0, &mut bits, 0, 0);
        let mut result = Err(Error::exc("ScreenShotError", "gdi32.dll.BitBlt() failed"));
        if dib != 0 && !bits.is_null() {
            let old = ffi::SelectObject(memdc, dib);
            // SRCCOPY | CAPTUREBLT
            if ffi::BitBlt(memdc, 0, 0, width, height, srcdc, left, top, 0x00CC_0020 | 0x4000_0000) != 0 {
                ffi::GdiFlush();
                let px = std::slice::from_raw_parts(bits as *const u8, (width as usize) * (height as usize) * 4);
                result = Ok(Img::from_bgrx(width as u32, height as u32, px));
            }
            ffi::SelectObject(memdc, old);
            ffi::DeleteObject(dib);
        }
        ffi::DeleteDC(memdc);
        ffi::ReleaseDC(0, srcdc);
        result
    }
}

/// Visible window bounds (left, top, width, height) in physical pixels,
/// without the invisible Windows 10/11 drop-shadow margins.
pub fn get_window_bounds(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    let wr = window_rect(hwnd)?;
    let (mut left, mut top, mut right, mut bottom) = (wr.left, wr.top, wr.right, wr.bottom);
    let mut dr = RECT::default();
    // DWMWA_EXTENDED_FRAME_BOUNDS
    let res = unsafe { ffi::DwmGetWindowAttribute(hwnd, 9, &mut dr as *mut RECT as *mut c_void, 16) };
    if res == 0 && dr.right > dr.left && dr.bottom > dr.top {
        (left, top, right, bottom) = (dr.left, dr.top, dr.right, dr.bottom);
    }
    let (w, h) = (right - left, bottom - top);
    (w > 0 && h > 0).then_some((left, top, w, h))
}

fn print_window(hwnd: isize, full_w: i32, full_h: i32) -> Option<Img> {
    unsafe {
        let hdc_win = ffi::GetWindowDC(hwnd);
        if hdc_win == 0 {
            return None;
        }
        let hdc_mem = ffi::CreateCompatibleDC(hdc_win);
        let hbmp = if hdc_mem != 0 { ffi::CreateCompatibleBitmap(hdc_win, full_w, full_h) } else { 0 };
        let old = if hbmp != 0 { ffi::SelectObject(hdc_mem, hbmp) } else { 0 };
        let mut img = None;
        if hdc_mem != 0 && hbmp != 0 {
            // PW_RENDERFULLCONTENT, then the plain call
            let ok = ffi::PrintWindow(hwnd, hdc_mem, 2) != 0 || ffi::PrintWindow(hwnd, hdc_mem, 0) != 0;
            if ok {
                let mut bmi = ffi::BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<ffi::BITMAPINFOHEADER>() as u32,
                    biWidth: full_w,
                    biHeight: -full_h,
                    biPlanes: 1,
                    biBitCount: 32,
                    ..Default::default()
                };
                let mut buf = vec![0u8; (full_w as usize) * (full_h as usize) * 4];
                let got = ffi::GetDIBits(hdc_mem, hbmp, 0, full_h as u32, buf.as_mut_ptr() as *mut c_void, &mut bmi, 0);
                if got > 0 {
                    img = Some(Img::from_bgrx(full_w as u32, full_h as u32, &buf));
                }
            }
        }
        if old != 0 {
            ffi::SelectObject(hdc_mem, old);
        }
        if hbmp != 0 {
            ffi::DeleteObject(hbmp);
        }
        if hdc_mem != 0 {
            ffi::DeleteDC(hdc_mem);
        }
        ffi::ReleaseDC(hwnd, hdc_win);
        img
    }
}

/// Capture a single window cleanly as (image, (left, top, w, h), method).
///
/// Tries PrintWindow(PW_RENDERFULLCONTENT) first so covered/background windows
/// can be captured without bringing them to the foreground. Falls back to a
/// screen crop over the visible window rectangle if PrintWindow fails or
/// returns a uniformly blank image.
pub fn capture_window_image(hwnd: isize) -> Res<(Img, (i32, i32, i32, i32), &'static str)> {
    let Some(wr) = window_rect(hwnd) else {
        return Err(Error::runtime(format!("Window {hwnd} has no valid rectangle")));
    };
    let (full_w, full_h) = (wr.right - wr.left, wr.bottom - wr.top);
    if full_w <= 0 || full_h <= 0 {
        return Err(Error::runtime(format!("Window {hwnd} has empty bounds")));
    }
    let mut vis = get_window_bounds(hwnd).unwrap_or((wr.left, wr.top, full_w, full_h));
    let (v_left, v_top, v_w, v_h) = vis;

    let mut img = None;
    if let Some(mut raw) = print_window(hwnd, full_w, full_h) {
        let cl = (v_left - wr.left).max(0);
        let ct = (v_top - wr.top).max(0);
        let cr = full_w.min(cl + v_w);
        let cb = full_h.min(ct + v_h);
        if cr > cl && cb > ct {
            raw = raw.crop(cl as u32, ct as u32, cr as u32, cb as u32);
            vis = (wr.left + cl, wr.top + ct, cr - cl, cb - ct);
        }
        if !raw.is_blank() {
            img = Some(raw);
        }
    }
    match img {
        Some(img) => Ok((img, vis, "print_window")),
        None => {
            let img = grab_screen(v_left, v_top, v_w, v_h)?;
            let vis = (v_left, v_top, img.w as i32, img.h as i32);
            Ok((img, vis, "screen_crop"))
        }
    }
}

// ---------------------------------------------------------------- clicks that keep the user's focus

const WM_ACTIVATE: u32 = 0x0006;
const WM_SETFOCUS: u32 = 0x0007;
const WM_KILLFOCUS: u32 = 0x0008;
const WM_ACTIVATEAPP: u32 = 0x001C;
const WM_MOUSEMOVE: u32 = 0x0200;

/// (down, up, double-click, MK_* flag) messages of a mouse button.
fn button_messages(button: &str) -> Option<(u32, u32, u32, usize)> {
    match button {
        "left" => Some((0x0201, 0x0202, 0x0203, 0x0001)),
        "right" => Some((0x0204, 0x0205, 0x0206, 0x0002)),
        "middle" => Some((0x0207, 0x0208, 0x0209, 0x0010)),
        _ => None,
    }
}

pub fn make_lparam(x: i32, y: i32) -> isize {
    ((((y as u32) & 0xFFFF) << 16) | ((x as u32) & 0xFFFF)) as isize
}

/// Walk down to the deepest visible enabled child HWND at (screen_x, screen_y).
fn resolve_child_target(hwnd: isize, screen_x: i32, screen_y: i32) -> (isize, i32, i32) {
    let mut cur = hwnd;
    // CWP_SKIPINVISIBLE | CWP_SKIPDISABLED | CWP_SKIPTRANSPARENT
    let flags = 0x0001 | 0x0002 | 0x0004;
    for _ in 0..16 {
        let mut pt = POINT { x: screen_x, y: screen_y };
        if unsafe { ffi::ScreenToClient(cur, &mut pt) } == 0 {
            break;
        }
        let child = unsafe { ffi::ChildWindowFromPointEx(cur, pt, flags) };
        if child == 0 || child == cur {
            return (cur, pt.x, pt.y);
        }
        cur = child;
    }
    let mut pt = POINT { x: screen_x, y: screen_y };
    unsafe { ffi::ScreenToClient(cur, &mut pt) };
    (cur, pt.x, pt.y)
}

fn post(hwnd: isize, msg: u32, wparam: usize, lparam: isize) {
    unsafe { ffi::PostMessageW(hwnd, msg, wparam, lparam) };
}

/// Post mouse click messages directly to a window without moving the mouse or stealing focus.
pub fn post_background_click(hwnd: isize, screen_x: i32, screen_y: i32, button: &str, clicks: i64) -> Res<(isize, (i32, i32))> {
    let Some((down_msg, up_msg, dbl_msg, mk)) = button_messages(button) else {
        return Err(Error::value(format!("Invalid mouse button: {button}")));
    };
    if !is_window(hwnd) {
        return Err(Error::runtime(format!("Window {hwnd} is no longer valid")));
    }
    let (target, cx, cy) = resolve_child_target(hwnd, screen_x, screen_y);
    let lparam = make_lparam(cx, cy);
    post(target, WM_MOUSEMOVE, 0, lparam);
    let n_clicks = clicks.max(1);
    for i in 0..n_clicks {
        if i == 1 && n_clicks == 2 {
            post(target, dbl_msg, mk, lparam);
            post(target, up_msg, 0, lparam);
        } else {
            post(target, down_msg, mk, lparam);
            sleep(0.02);
            post(target, up_msg, 0, lparam);
        }
        if i + 1 < n_clicks {
            sleep(0.05);
        }
    }
    Ok((target, (cx, cy)))
}

/// Click a window that reads the real cursor position, without activating it.
///
/// Some programs (Unity games) ignore the coordinates carried by posted mouse
/// messages: they take the position from the real cursor and only handle clicks
/// while they believe they are the active window. So the cursor jumps to the
/// point for a fraction of a second and comes back, and the window is only
/// *told* it is active: the foreground window and the keyboard focus stay
/// where the user has them.
pub fn cursor_jump_click(hwnd: isize, screen_x: i32, screen_y: i32, button: &str, clicks: i64) -> Res<(isize, (i32, i32))> {
    let (settle, hold, linger) = (0.06, 0.05, 0.08);
    let Some((down_msg, up_msg, _, mk)) = button_messages(button) else {
        return Err(Error::value(format!("Invalid mouse button: {button}")));
    };
    if !is_window(hwnd) {
        return Err(Error::runtime(format!("Window {hwnd} is no longer valid")));
    }
    if unsafe { ffi::IsIconic(hwnd) } != 0 {
        return Err(Error::runtime(format!("Window {hwnd} is minimized; restore it first")));
    }
    let (sx, sy) = (screen_x, screen_y);
    let (target, cx, cy) = resolve_child_target(hwnd, sx, sy);
    let lparam = make_lparam(cx, cy);

    // Moving the cursor while the user holds a button would drag whatever they hold.
    let deadline = now() + 3.0;
    while [0x01, 0x02, 0x04].iter().any(|vk| unsafe { ffi::GetAsyncKeyState(*vk) } as u16 & 0x8000 != 0) {
        if now() >= deadline {
            return Err(Error::runtime("The user is holding a mouse button; try again in a moment"));
        }
        sleep(0.05);
    }

    let mut pt = POINT::default();
    let prev_pos = (unsafe { ffi::GetCursorPos(&mut pt) } != 0).then_some((pt.x, pt.y));
    let pretend_active = foreground() != hwnd;
    unsafe { ffi::SetCursorPos(sx, sy) };
    if pretend_active {
        post(hwnd, WM_ACTIVATEAPP, 1, 0);
        post(hwnd, WM_ACTIVATE, 1, 0);
        post(hwnd, WM_SETFOCUS, 0, 0);
    }
    post(target, WM_MOUSEMOVE, 0, lparam);
    sleep(settle);
    for i in 0..clicks.max(1) {
        if i > 0 {
            sleep(0.08);
        }
        unsafe { ffi::SetCursorPos(sx, sy) }; // in case the user nudged the mouse meanwhile
        post(target, down_msg, mk, lparam);
        sleep(hold);
        unsafe { ffi::SetCursorPos(sx, sy) };
        post(target, up_msg, 0, lparam);
    }
    sleep(linger); // the click is read on the window's next frame
    if let Some((px, py)) = prev_pos {
        unsafe { ffi::SetCursorPos(px, py) };
    }
    if pretend_active && foreground() != hwnd {
        post(hwnd, WM_KILLFOCUS, 0, 0);
        post(hwnd, WM_ACTIVATE, 0, 0);
        post(hwnd, WM_ACTIVATEAPP, 0, 0);
    }
    Ok((target, (cx, cy)))
}

/// Record current foreground HWND and cursor position (x, y).
pub fn save_foreground_state() -> (isize, Option<(i32, i32)>) {
    let hwnd = foreground();
    let mut pt = POINT::default();
    let pos = (unsafe { ffi::GetCursorPos(&mut pt) } != 0).then_some((pt.x, pt.y));
    (hwnd, pos)
}

/// Best-effort restore of cursor position and original foreground window.
pub fn restore_foreground_state(prev_hwnd: isize, prev_pos: Option<(i32, i32)>, target_hwnd: Option<isize>) {
    unsafe {
        if let Some((x, y)) = prev_pos {
            ffi::SetCursorPos(x, y);
        }
        if prev_hwnd != 0 && Some(prev_hwnd) != target_hwnd && ffi::IsWindow(prev_hwnd) != 0 && ffi::IsIconic(prev_hwnd) == 0 {
            ffi::SetForegroundWindow(prev_hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lparam_packs_client_coordinates() {
        assert_eq!(make_lparam(40, 60), (60 << 16) | 40);
        assert_eq!(make_lparam(-1, 2), (2 << 16) | 0xFFFF);
    }

    #[test]
    fn button_messages_cover_three_buttons() {
        assert_eq!(button_messages("left"), Some((0x0201, 0x0202, 0x0203, 1)));
        assert_eq!(button_messages("right").unwrap().3, 2);
        assert_eq!(button_messages("middle").unwrap().3, 0x10);
        assert!(button_messages("sideways").is_none());
    }
}
