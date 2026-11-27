//! Raw Win32 declarations used by the server (user32, gdi32, kernel32, ...).
//!
//! Plain FFI keeps handles as `isize` and mirrors the calls the Python
//! version makes through ctypes one to one.
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms, dead_code)]

use std::ffi::c_void;

pub type HWND = isize;
pub type HDC = isize;
pub type HANDLE = isize;
pub type HGDIOBJ = isize;
pub type BOOL = i32;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SIZE {
    pub cx: i32,
    pub cy: i32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SYSTEMTIME {
    pub wYear: u16,
    pub wMonth: u16,
    pub wDayOfWeek: u16,
    pub wDay: u16,
    pub wHour: u16,
    pub wMinute: u16,
    pub wSecond: u16,
    pub wMilliseconds: u16,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct BITMAPINFOHEADER {
    pub biSize: u32,
    pub biWidth: i32,
    pub biHeight: i32,
    pub biPlanes: u16,
    pub biBitCount: u16,
    pub biCompression: u32,
    pub biSizeImage: u32,
    pub biXPelsPerMeter: i32,
    pub biYPelsPerMeter: i32,
    pub biClrUsed: u32,
    pub biClrImportant: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct MONITORINFO {
    pub cbSize: u32,
    pub rcMonitor: RECT,
    pub rcWork: RECT,
    pub dwFlags: u32,
}

pub type WNDENUMPROC = unsafe extern "system" fn(HWND, isize) -> BOOL;
pub type MONITORENUMPROC = unsafe extern "system" fn(isize, HDC, *mut RECT, isize) -> BOOL;

#[link(name = "user32")]
extern "system" {
    pub fn GetCursorPos(p: *mut POINT) -> BOOL;
    pub fn SetCursorPos(x: i32, y: i32) -> BOOL;
    pub fn mouse_event(dwFlags: u32, dx: i32, dy: i32, dwData: u32, dwExtraInfo: usize);
    pub fn keybd_event(bVk: u8, bScan: u8, dwFlags: u32, dwExtraInfo: usize);
    pub fn VkKeyScanA(ch: i8) -> i16;
    pub fn GetSystemMetrics(n: i32) -> i32;
    pub fn EnumWindows(cb: WNDENUMPROC, lparam: isize) -> BOOL;
    pub fn IsWindowVisible(hwnd: HWND) -> BOOL;
    pub fn GetWindowTextLengthW(hwnd: HWND) -> i32;
    pub fn GetWindowTextW(hwnd: HWND, buf: *mut u16, n: i32) -> i32;
    pub fn GetWindowLongW(hwnd: HWND, index: i32) -> i32;
    pub fn GetWindowRect(hwnd: HWND, r: *mut RECT) -> BOOL;
    pub fn IsIconic(hwnd: HWND) -> BOOL;
    pub fn IsWindow(hwnd: HWND) -> BOOL;
    pub fn ShowWindow(hwnd: HWND, cmd: i32) -> BOOL;
    pub fn SetForegroundWindow(hwnd: HWND) -> BOOL;
    pub fn GetForegroundWindow() -> HWND;
    pub fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    pub fn GetWindowDC(hwnd: HWND) -> HDC;
    pub fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    pub fn PrintWindow(hwnd: HWND, hdc: HDC, flags: u32) -> BOOL;
    pub fn ScreenToClient(hwnd: HWND, p: *mut POINT) -> BOOL;
    pub fn ChildWindowFromPointEx(hwnd: HWND, p: POINT, flags: u32) -> HWND;
    pub fn PostMessageW(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> BOOL;
    pub fn GetAsyncKeyState(vk: i32) -> i16;
    pub fn EnumDisplayMonitors(hdc: HDC, clip: *const RECT, cb: MONITORENUMPROC, data: isize) -> BOOL;
    pub fn GetMonitorInfoW(hmon: isize, info: *mut MONITORINFO) -> BOOL;
    pub fn OpenClipboard(hwnd: HWND) -> BOOL;
    pub fn CloseClipboard() -> BOOL;
    pub fn EmptyClipboard() -> BOOL;
    pub fn GetClipboardData(format: u32) -> HANDLE;
    pub fn SetClipboardData(format: u32, h: HANDLE) -> HANDLE;
    pub fn CreateWindowExA(
        ex: u32, class: *const u8, name: *const u8, style: u32, x: i32, y: i32, w: i32, h: i32,
        parent: HWND, menu: isize, inst: isize, param: *mut c_void,
    ) -> HWND;
    pub fn DestroyWindow(hwnd: HWND) -> BOOL;
    pub fn SetProcessDPIAware() -> BOOL;
    pub fn RegisterClassW(class: *const WNDCLASSW) -> u16;
    pub fn CreateWindowExW(
        ex: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32,
        parent: HWND, menu: isize, inst: isize, param: *mut c_void,
    ) -> HWND;
    pub fn DefWindowProcW(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize;
    pub fn GetMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32) -> BOOL;
    pub fn TranslateMessage(msg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(msg: *const MSG) -> isize;
    pub fn IsDialogMessageW(hwnd: HWND, msg: *mut MSG) -> BOOL;
    pub fn PostQuitMessage(code: i32);
    pub fn SetTimer(hwnd: HWND, id: usize, ms: u32, func: isize) -> usize;
    pub fn SetWindowTextW(hwnd: HWND, text: *const u16) -> BOOL;
    pub fn SendMessageW(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize;
    pub fn LoadCursorW(inst: isize, name: *const u16) -> isize;
    pub fn UpdateWindow(hwnd: HWND) -> BOOL;
    pub fn AdjustWindowRectEx(rect: *mut RECT, style: u32, menu: BOOL, ex: u32) -> BOOL;
}

pub type WNDPROC = unsafe extern "system" fn(HWND, u32, usize, isize) -> isize;

#[repr(C)]
pub struct WNDCLASSW {
    pub style: u32,
    pub lpfnWndProc: Option<WNDPROC>,
    pub cbClsExtra: i32,
    pub cbWndExtra: i32,
    pub hInstance: isize,
    pub hIcon: isize,
    pub hCursor: isize,
    pub hbrBackground: isize,
    pub lpszMenuName: *const u16,
    pub lpszClassName: *const u16,
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: usize,
    pub lParam: isize,
    pub time: u32,
    pub pt: POINT,
    pub lPrivate: u32,
}

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
    pub fn CreateCompatibleBitmap(hdc: HDC, w: i32, h: i32) -> HGDIOBJ;
    pub fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    pub fn GetDIBits(
        hdc: HDC, bmp: HGDIOBJ, start: u32, lines: u32, bits: *mut c_void,
        bmi: *mut BITMAPINFOHEADER, usage: u32,
    ) -> i32;
    pub fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    pub fn DeleteDC(hdc: HDC) -> BOOL;
    pub fn BitBlt(dst: HDC, x: i32, y: i32, w: i32, h: i32, src: HDC, sx: i32, sy: i32, rop: u32) -> BOOL;
    pub fn CreateDIBSection(
        hdc: HDC, bmi: *const BITMAPINFOHEADER, usage: u32, bits: *mut *mut c_void,
        section: HANDLE, offset: u32,
    ) -> HGDIOBJ;
    pub fn GdiFlush() -> BOOL;
    pub fn CreateFontW(
        height: i32, width: i32, esc: i32, orient: i32, weight: i32, italic: u32, underline: u32,
        strike: u32, charset: u32, out_prec: u32, clip_prec: u32, quality: u32, pitch: u32,
        face: *const u16,
    ) -> HGDIOBJ;
    pub fn SetTextColor(hdc: HDC, color: u32) -> u32;
    pub fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    pub fn TextOutW(hdc: HDC, x: i32, y: i32, s: *const u16, n: i32) -> BOOL;
    pub fn GetTextExtentPoint32W(hdc: HDC, s: *const u16, n: i32, size: *mut SIZE) -> BOOL;
}

#[link(name = "dwmapi")]
extern "system" {
    pub fn DwmGetWindowAttribute(hwnd: HWND, attr: u32, out: *mut c_void, size: u32) -> i32;
}

#[link(name = "shcore")]
extern "system" {
    pub fn SetProcessDpiAwareness(value: i32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    pub fn ShellExecuteW(
        hwnd: HWND, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32,
    ) -> isize;
}

#[repr(C)]
pub struct WIN32_FIND_DATAW {
    pub dwFileAttributes: u32,
    pub ftCreationTime: [u32; 2],
    pub ftLastAccessTime: [u32; 2],
    pub ftLastWriteTime: [u32; 2],
    pub nFileSizeHigh: u32,
    pub nFileSizeLow: u32,
    /// the reparse tag when the entry is a reparse point
    pub dwReserved0: u32,
    pub dwReserved1: u32,
    pub cFileName: [u16; 260],
    pub cAlternateFileName: [u16; 14],
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> isize;
    pub fn FindFirstFileW(name: *const u16, data: *mut WIN32_FIND_DATAW) -> HANDLE;
    pub fn FindClose(h: HANDLE) -> BOOL;
    pub fn GetLocalTime(st: *mut SYSTEMTIME);
    pub fn GetSystemTime(st: *mut SYSTEMTIME);
    pub fn GlobalAlloc(flags: u32, bytes: usize) -> HANDLE;
    pub fn GlobalLock(h: HANDLE) -> *mut c_void;
    pub fn GlobalUnlock(h: HANDLE) -> BOOL;
    pub fn GlobalFree(h: HANDLE) -> HANDLE;
    pub fn MultiByteToWideChar(cp: u32, flags: u32, src: *const u8, n: i32, dst: *mut u16, dn: i32) -> i32;
    pub fn GetUserDefaultUILanguage() -> u16;
    pub fn LCIDToLocaleName(lcid: u32, name: *mut u16, n: i32, flags: u32) -> i32;
    pub fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    pub fn GetLastError() -> u32;
    pub fn FormatMessageW(
        flags: u32, source: *const c_void, id: u32, lang: u32, buf: *mut u16, size: u32,
        args: *mut c_void,
    ) -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    /// RtlGenRandom
    pub fn SystemFunction036(buf: *mut u8, len: u32) -> u8;
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Fill `buf` with cryptographically secure random bytes.
pub fn random_fill(buf: &mut [u8]) {
    unsafe {
        SystemFunction036(buf.as_mut_ptr(), buf.len() as u32);
    }
}

/// `secrets.token_hex(n)`.
pub fn token_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    random_fill(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Start a child program without a console window of its own (CREATE_NO_WINDOW).
pub fn no_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}

pub fn local_time() -> SYSTEMTIME {
    let mut st = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut st) };
    st
}

pub fn system_time() -> SYSTEMTIME {
    let mut st = SYSTEMTIME::default();
    unsafe { GetSystemTime(&mut st) };
    st
}

/// Text of a Win32 error code, trimmed the way CPython trims it
/// (trailing whitespace and ASCII dots removed).
pub fn win_error_text(code: u32) -> String {
    let mut buf = [0u16; 1024];
    // FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS
    let n = unsafe {
        FormatMessageW(0x1000 | 0x200, std::ptr::null(), code, 0, buf.as_mut_ptr(), buf.len() as u32, std::ptr::null_mut())
    } as usize;
    let mut s = String::from_utf16_lossy(&buf[..n]);
    while let Some(c) = s.chars().last() {
        if c <= ' ' || c == '.' {
            s.pop();
        } else {
            break;
        }
    }
    s
}
