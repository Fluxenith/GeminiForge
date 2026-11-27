//! The Win32 confirmation popup shown by `geminiforge-server approve` (see approve.rs).

use super::shown_args;
use crate::ffi::{self, HWND};
use crate::lang;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

const ID_ALLOW: usize = 100;
const ID_DENY: usize = 101;
const ID_ALLOW_TIMED: usize = 102;
const IDCANCEL: usize = 2;

const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_SETFONT: u32 = 0x0030;
const WM_COMMAND: u32 = 0x0111;
const WM_TIMER: u32 = 0x0113;

const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;

static RESULT: AtomicI32 = AtomicI32::new(1);
static LEFT: AtomicI32 = AtomicI32::new(0);
static COUNT_LABEL: AtomicIsize = AtomicIsize::new(0);

fn countdown_text(left: i32) -> Vec<u16> {
    ffi::wide(&lang::t("countdown", &[("n", &left.to_string())]))
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    let finish = |code: i32| {
        RESULT.store(code, Ordering::SeqCst);
        ffi::DestroyWindow(hwnd);
    };
    match msg {
        WM_COMMAND => {
            match wparam & 0xFFFF {
                ID_ALLOW => finish(0),
                ID_ALLOW_TIMED => finish(2),
                ID_DENY | IDCANCEL => finish(1),
                _ => {}
            }
            0
        }
        WM_TIMER => {
            let left = LEFT.fetch_sub(1, Ordering::SeqCst) - 1;
            if left <= 0 {
                finish(1);
            } else {
                ffi::SetWindowTextW(COUNT_LABEL.load(Ordering::SeqCst), countdown_text(left).as_ptr());
            }
            0
        }
        WM_CLOSE => {
            finish(1);
            0
        }
        WM_DESTROY => {
            ffi::PostQuitMessage(0);
            0
        }
        _ => ffi::DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn font(height: i32, weight: i32, face: &str) -> isize {
    let face = ffi::wide(face);
    unsafe { ffi::CreateFontW(-height, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) }
}

/// Show the popup and return its exit code.
pub fn show(tool: &str, raw_args: &str, timeout: i32) -> i32 {
    let args = shown_args(raw_args);
    let lines = (args.matches('\n').count() as i32 + 2).clamp(4, 16);
    let (pad, width) = (18, 560);
    let box_h = lines * 17 + 10;
    let y_box = 48;
    let y_count = y_box + box_h + 6;
    let y_buttons = y_count + 30;
    let client_h = y_buttons + 34 + 16;
    RESULT.store(1, Ordering::SeqCst);
    LEFT.store(timeout, Ordering::SeqCst);
    unsafe {
        let instance = ffi::GetModuleHandleW(std::ptr::null());
        let class = ffi::wide("GeminiForgeApprove");
        let wc = ffi::WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: 0,
            hCursor: ffi::LoadCursorW(0, 32512 as *const u16), // IDC_ARROW
            hbrBackground: 16,                                 // COLOR_BTNFACE + 1
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        if ffi::RegisterClassW(&wc) == 0 {
            return 1;
        }
        // window size for the wanted client area
        let mut r = ffi::RECT { left: 0, top: 0, right: width, bottom: client_h };
        ffi::AdjustWindowRectEx(&mut r, WS_CAPTION | WS_SYSMENU, 0, WS_EX_TOPMOST);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let x = (ffi::GetSystemMetrics(0) - w) / 2;
        let y = (ffi::GetSystemMetrics(1) - h) / 3;
        let title = ffi::wide(&lang::t("dialogTitle", &[]));
        let hwnd = ffi::CreateWindowExW(
            WS_EX_TOPMOST, class.as_ptr(), title.as_ptr(), WS_CAPTION | WS_SYSMENU, x, y, w, h, 0, 0, instance,
            std::ptr::null_mut(),
        );
        if hwnd == 0 {
            return 1;
        }
        let (normal, bold, mono) = (font(15, 400, "Segoe UI"), font(18, 700, "Segoe UI"), font(14, 400, "Consolas"));
        let child = |class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: usize, f: isize| -> HWND {
            let (c, t) = (ffi::wide(class), ffi::wide(text));
            let ctl = ffi::CreateWindowExW(
                0, c.as_ptr(), t.as_ptr(), WS_CHILD | WS_VISIBLE | style, x, y, w, h, hwnd, id as isize, instance,
                std::ptr::null_mut(),
            );
            ffi::SendMessageW(ctl, WM_SETFONT, f as usize, 1);
            ctl
        };
        child("STATIC", &lang::t("dialogAsk", &[("tool", tool)]), 0, pad, 16, width - 2 * pad, 26, 0, bold);
        child(
            "EDIT",
            &args.replace("\r\n", "\n").replace('\n', "\r\n"),
            WS_BORDER | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
            pad, y_box, width - 2 * pad, box_h, 0, mono,
        );
        let count = child("STATIC", &lang::t("countdown", &[("n", &timeout.to_string())]), 0, pad, y_count, width - 2 * pad, 22, 0, normal);
        COUNT_LABEL.store(count, Ordering::SeqCst);
        child("BUTTON", &lang::t("allow", &[]), WS_TABSTOP, pad, y_buttons, 96, 32, ID_ALLOW, normal);
        child("BUTTON", &lang::t("allowTimed", &[]), WS_TABSTOP, pad + 104, y_buttons, 250, 32, ID_ALLOW_TIMED, normal);
        child("BUTTON", &lang::t("deny", &[]), WS_TABSTOP, width - pad - 96, y_buttons, 96, 32, ID_DENY, normal);
        ffi::SetTimer(hwnd, 1, 1000, 0);
        ffi::ShowWindow(hwnd, 5);
        ffi::UpdateWindow(hwnd);
        ffi::SetForegroundWindow(hwnd);
        let mut msg: ffi::MSG = std::mem::zeroed();
        while ffi::GetMessageW(&mut msg, 0, 0, 0) > 0 {
            if ffi::IsDialogMessageW(hwnd, &mut msg) == 0 {
                ffi::TranslateMessage(&msg);
                ffi::DispatchMessageW(&msg);
            }
        }
        for f in [normal, bold, mono] {
            ffi::DeleteObject(f);
        }
    }
    RESULT.load(Ordering::SeqCst)
}
