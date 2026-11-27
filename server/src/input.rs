//! Mouse and keyboard input, a port of the pyautogui calls server.py makes.
//!
//! Same Win32 calls (SetCursorPos, mouse_event, keybd_event), same fail-safe
//! (mouse in a screen corner raises), same 0.05 s pause after each call.

use crate::ffi;
use crate::py::{self, Error, Res};
use crate::state::sleep;
use std::sync::OnceLock;

/// `pyautogui.PAUSE`
pub const PAUSE: f64 = 0.05;
const MINIMUM_DURATION: f64 = 0.1;
const MINIMUM_SLEEP: f64 = 0.05;

const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const KEYEVENTF_KEYUP: u32 = 0x0002;

static CORNERS: OnceLock<[(i32, i32); 4]> = OnceLock::new();

/// `pyautogui.size()`: the primary screen.
pub fn size() -> (i32, i32) {
    unsafe { (ffi::GetSystemMetrics(0), ffi::GetSystemMetrics(1)) }
}

/// The fail-safe corners are fixed when the server starts, as pyautogui fixes them on import.
fn corners() -> &'static [(i32, i32); 4] {
    CORNERS.get_or_init(|| {
        let (right, bottom) = size();
        [(0, 0), (0, bottom - 1), (right - 1, 0), (right - 1, bottom - 1)]
    })
}

pub fn init() {
    corners();
}

/// `pyautogui.position()`
pub fn position() -> (i32, i32) {
    let mut p = ffi::POINT::default();
    unsafe { ffi::GetCursorPos(&mut p) };
    (p.x, p.y)
}

pub fn fail_safe_check() -> Res<()> {
    if corners().contains(&position()) {
        return Err(Error::FailSafe);
    }
    Ok(())
}

fn pause() {
    sleep(PAUSE);
}

fn send_mouse_event(ev: u32, x: i32, y: i32, data: u32) {
    let (width, height) = size();
    let cx = if width != 0 { (65536i64 * x as i64).div_euclid(width as i64) + 1 } else { 0 };
    let cy = if height != 0 { (65536i64 * y as i64).div_euclid(height as i64) + 1 } else { 0 };
    unsafe { ffi::mouse_event(ev, cx as i32, cy as i32, data, 0) };
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// `_normalizeButton`
pub fn normalize_button(button: &str) -> Res<Button> {
    let swapped = || unsafe { ffi::GetSystemMetrics(23) != 0 };
    match button.to_lowercase().as_str() {
        "left" => Ok(Button::Left),
        "middle" => Ok(Button::Middle),
        "right" => Ok(Button::Right),
        "primary" => Ok(if swapped() { Button::Right } else { Button::Left }),
        "secondary" => Ok(if swapped() { Button::Left } else { Button::Right }),
        _ => Err(Error::exc(
            "PyAutoGUIException",
            "button argument must be one of ('left', 'middle', 'right', 'primary', 'secondary', 1, 2, 3)",
        )),
    }
}

/// `_mouseMoveDrag`: instant, or in steps when `duration` is over 0.1 s.
fn mouse_move_drag(x: i32, y: i32, duration: f64) -> Res<()> {
    let (startx, starty) = position();
    let (width, height) = size();
    let mut steps: Vec<(f64, f64)> = vec![(x as f64, y as f64)];
    let mut sleep_amount = 0.0;
    if duration > MINIMUM_DURATION {
        let mut num_steps = width.max(height).max(1) as f64;
        sleep_amount = duration / num_steps;
        if sleep_amount < MINIMUM_SLEEP {
            num_steps = (duration / MINIMUM_SLEEP).trunc();
            sleep_amount = duration / num_steps;
        }
        let n_steps = num_steps as i64;
        steps = (0..n_steps)
            .map(|n| {
                let t = n as f64 / num_steps;
                ((x - startx) as f64 * t + startx as f64, (y - starty) as f64 * t + starty as f64)
            })
            .collect();
        steps.push((x as f64, y as f64));
    }
    let many = steps.len() > 1;
    let mut last = (x, y);
    for (fx, fy) in steps {
        if many {
            sleep(sleep_amount);
        }
        let (tx, ty) = (py::round(fx) as i32, py::round(fy) as i32);
        last = (tx, ty);
        if !corners().contains(&(tx, ty)) {
            fail_safe_check()?;
        }
        unsafe { ffi::SetCursorPos(tx, ty) };
    }
    if !corners().contains(&last) {
        fail_safe_check()?;
    }
    Ok(())
}

/// `pyautogui.click(x, y, clicks=clicks, interval=interval, button=button)`
pub fn click(x: i32, y: i32, clicks: i64, interval: f64, button: &str) -> Res<()> {
    fail_safe_check()?;
    let button = normalize_button(button)?;
    mouse_move_drag(x, y, 0.0)?;
    let ev = match button {
        Button::Left => MOUSEEVENTF_LEFTDOWN | MOUSEEVENTF_LEFTUP,
        Button::Middle => MOUSEEVENTF_MIDDLEDOWN | MOUSEEVENTF_MIDDLEUP,
        Button::Right => MOUSEEVENTF_RIGHTDOWN | MOUSEEVENTF_RIGHTUP,
    };
    for _ in 0..clicks.max(0) {
        fail_safe_check()?;
        send_mouse_event(ev, x, y, 0);
        sleep(interval);
    }
    pause();
    Ok(())
}

/// `pyautogui.moveTo(x, y, duration=duration)`
pub fn move_to(x: i32, y: i32, duration: f64) -> Res<()> {
    fail_safe_check()?;
    mouse_move_drag(x, y, duration)?;
    pause();
    Ok(())
}

fn mouse_button(down: bool) -> Res<()> {
    // mouseDown / mouseUp with _pause=False, at the current position
    fail_safe_check()?;
    let (x, y) = position();
    mouse_move_drag(x, y, 0.0)?;
    send_mouse_event(if down { MOUSEEVENTF_LEFTDOWN } else { MOUSEEVENTF_LEFTUP }, x, y, 0);
    Ok(())
}

/// `pyautogui.dragTo(x, y, duration=duration, button="left")`
pub fn drag_to(x: i32, y: i32, duration: f64) -> Res<()> {
    fail_safe_check()?;
    mouse_button(true)?;
    mouse_move_drag(x, y, duration)?;
    mouse_button(false)?;
    pause();
    Ok(())
}

/// `pyautogui.scroll(clicks)` at the current mouse position.
pub fn scroll(clicks: i32) -> Res<()> {
    fail_safe_check()?;
    let (x, y) = position();
    let (width, height) = size();
    let (x, y) = (x.clamp(0, (width - 1).max(0)), y.clamp(0, (height - 1).max(0)));
    send_mouse_event(MOUSEEVENTF_WHEEL, x, y, clicks as u32);
    pause();
    Ok(())
}

const NAMED_KEYS: &[(&str, u8)] = &[
    ("backspace", 0x08), ("\x08", 0x08), ("super", 0x5B), ("tab", 0x09), ("\t", 0x09), ("clear", 0x0c),
    ("enter", 0x0d), ("\n", 0x0d), ("return", 0x0d), ("shift", 0x10), ("ctrl", 0x11), ("alt", 0x12),
    ("pause", 0x13), ("capslock", 0x14), ("kana", 0x15), ("hanguel", 0x15), ("hangul", 0x15),
    ("junja", 0x17), ("final", 0x18), ("hanja", 0x19), ("kanji", 0x19), ("esc", 0x1b), ("escape", 0x1b),
    ("convert", 0x1c), ("nonconvert", 0x1d), ("accept", 0x1e), ("modechange", 0x1f), ("space", 0x20),
    ("pgup", 0x21), ("pgdn", 0x22), ("pageup", 0x21), ("pagedown", 0x22), ("end", 0x23), ("home", 0x24),
    ("left", 0x25), ("up", 0x26), ("right", 0x27), ("down", 0x28), ("select", 0x29), ("print", 0x2a),
    ("execute", 0x2b), ("prtsc", 0x2c), ("prtscr", 0x2c), ("prntscrn", 0x2c), ("printscreen", 0x2c),
    ("insert", 0x2d), ("del", 0x2e), ("delete", 0x2e), ("help", 0x2f), ("win", 0x5b), ("winleft", 0x5b),
    ("winright", 0x5c), ("apps", 0x5d), ("sleep", 0x5f), ("num0", 0x60), ("num1", 0x61), ("num2", 0x62),
    ("num3", 0x63), ("num4", 0x64), ("num5", 0x65), ("num6", 0x66), ("num7", 0x67), ("num8", 0x68),
    ("num9", 0x69), ("multiply", 0x6a), ("add", 0x6b), ("separator", 0x6c), ("subtract", 0x6d),
    ("decimal", 0x6e), ("divide", 0x6f), ("f1", 0x70), ("f2", 0x71), ("f3", 0x72), ("f4", 0x73),
    ("f5", 0x74), ("f6", 0x75), ("f7", 0x76), ("f8", 0x77), ("f9", 0x78), ("f10", 0x79), ("f11", 0x7a),
    ("f12", 0x7b), ("f13", 0x7c), ("f14", 0x7d), ("f15", 0x7e), ("f16", 0x7f), ("f17", 0x80),
    ("f18", 0x81), ("f19", 0x82), ("f20", 0x83), ("f21", 0x84), ("f22", 0x85), ("f23", 0x86),
    ("f24", 0x87), ("numlock", 0x90), ("scrolllock", 0x91), ("shiftleft", 0xa0), ("shiftright", 0xa1),
    ("ctrlleft", 0xa2), ("ctrlright", 0xa3), ("altleft", 0xa4), ("altright", 0xa5),
    ("browserback", 0xa6), ("browserforward", 0xa7), ("browserrefresh", 0xa8), ("browserstop", 0xa9),
    ("browsersearch", 0xaa), ("browserfavorites", 0xab), ("browserhome", 0xac), ("volumemute", 0xad),
    ("volumedown", 0xae), ("volumeup", 0xaf), ("nexttrack", 0xb0), ("prevtrack", 0xb1), ("stop", 0xb2),
    ("playpause", 0xb3), ("launchmail", 0xb4), ("launchmediaselect", 0xb5), ("launchapp1", 0xb6),
    ("launchapp2", 0xb7),
];

/// `keyboardMapping[key]`: (modifier bits, virtual key), or None for a key pyautogui ignores.
pub fn key_code(key: &str) -> Option<(u8, u8)> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        if (' '..='\x7f').contains(&c) {
            let code = unsafe { ffi::VkKeyScanA(c as u8 as i8) };
            if code == -1 {
                return None;
            }
            return Some(((code >> 8) as u8, (code & 0xff) as u8));
        }
    }
    NAMED_KEYS.iter().find(|(name, _)| *name == key).map(|(_, vk)| (0u8, *vk))
}

/// `pyautogui.isShiftCharacter`
fn is_shift_character(key: &str) -> bool {
    let has_cased = key.chars().any(|c| c.is_uppercase() || c.is_lowercase());
    let is_upper = has_cased && !key.chars().any(|c| c.is_lowercase());
    is_upper || (py::len(key) == 1 && "~!@#$%^&*()_+{}|:\"<>?".contains(key))
}

fn key_event(vk: u8, up: bool) {
    unsafe { ffi::keybd_event(vk, 0, if up { KEYEVENTF_KEYUP } else { 0 }, 0) };
}

fn key_action(key: &str, up: bool) {
    let Some((mods, vk)) = key_code(key) else { return };
    let shift = mods & 1 != 0 || is_shift_character(key);
    let (ctrl, alt) = (mods & 2 != 0, mods & 4 != 0);
    for (apply, vk_mod) in [(alt, 0x12u8), (ctrl, 0x11), (shift, 0x10)] {
        if apply {
            key_event(vk_mod, false);
        }
    }
    key_event(vk, up);
    for (apply, vk_mod) in [(shift, 0x10u8), (ctrl, 0x11), (alt, 0x12)] {
        if apply {
            key_event(vk_mod, true);
        }
    }
}

fn lowered(key: &str) -> String {
    if py::len(key) > 1 {
        key.to_lowercase()
    } else {
        key.to_string()
    }
}

fn press_inner(key: &str) -> Res<()> {
    fail_safe_check()?;
    let key = lowered(key);
    fail_safe_check()?;
    key_action(&key, false);
    key_action(&key, true);
    Ok(())
}

/// `pyautogui.press(key)`
pub fn press(key: &str) -> Res<()> {
    press_inner(key)?;
    pause();
    Ok(())
}

/// `pyautogui.hotkey(*keys)`
pub fn hotkey(keys: &[&str]) -> Res<()> {
    fail_safe_check()?;
    let keys: Vec<String> = keys.iter().map(|k| lowered(k)).collect();
    for k in &keys {
        key_action(k, false);
    }
    for k in keys.iter().rev() {
        key_action(k, true);
    }
    pause();
    Ok(())
}

/// `pyautogui.write(text, interval=interval)`
pub fn write(text: &str, interval: f64) -> Res<()> {
    fail_safe_check()?;
    let mut buf = [0u8; 4];
    for c in text.chars() {
        press_inner(c.encode_utf8(&mut buf))?;
        sleep(interval);
        fail_safe_check()?;
    }
    pause();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_map_like_pyautogui() {
        assert_eq!(key_code("enter"), Some((0, 0x0d)));
        assert_eq!(key_code("ctrl"), Some((0, 0x11)));
        assert_eq!(key_code("f12"), Some((0, 0x7b)));
        assert_eq!(key_code("\t"), Some((0, 0x09)));
        assert_eq!(key_code("command"), None);
        assert_eq!(key_code("nosuchkey"), None);
        assert_eq!(key_code("あ"), None);
        assert_eq!(key_code("a").map(|(_, vk)| vk), Some(0x41));
        assert!(is_shift_character("A"));
        assert!(is_shift_character("!"));
        assert!(!is_shift_character("a"));
        assert!(!is_shift_character("enter"));
        assert!(!is_shift_character("1"));
    }

    #[test]
    fn buttons_are_validated() {
        assert_eq!(normalize_button("LEFT").unwrap(), Button::Left);
        assert_eq!(normalize_button("middle").unwrap(), Button::Middle);
        assert!(normalize_button("sideways").is_err());
    }
}
