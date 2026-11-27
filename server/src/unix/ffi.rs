//! The operating-system calls the shared code needs, for macOS and Linux.
//!
//! Same names and shapes as the Windows `src/ffi.rs` (`SYSTEMTIME`,
//! `local_time`, `token_hex`, ...), so the shared modules do not need to
//! know which one they got.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_pointer_width = "64"))]
compile_error!("GeminiForge supports 64-bit macOS and Linux only");

/// The fields of Windows' SYSTEMTIME that the server reads.
#[derive(Default, Clone, Copy, Debug)]
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

/// `struct tm` of glibc, musl and macOS on 64-bit systems.
#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const u8,
}

extern "C" {
    fn tzset();
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    fn gmtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    fn kill(pid: i32, sig: i32) -> i32;
}

const SIGKILL: i32 = 9;

fn now_parts() -> (i64, u16) {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as i64, d.subsec_millis() as u16)
}

fn convert(f: unsafe extern "C" fn(*const i64, *mut Tm) -> *mut Tm) -> Option<SYSTEMTIME> {
    let (secs, ms) = now_parts();
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    if unsafe { f(&secs, &mut tm) }.is_null() {
        return None;
    }
    Some(SYSTEMTIME {
        wYear: (tm.tm_year + 1900) as u16,
        wMonth: (tm.tm_mon + 1) as u16,
        wDayOfWeek: tm.tm_wday as u16,
        wDay: tm.tm_mday as u16,
        wHour: tm.tm_hour as u16,
        wMinute: tm.tm_min as u16,
        wSecond: tm.tm_sec as u16,
        wMilliseconds: ms,
    })
}

/// The current time in UTC.
pub fn system_time() -> SYSTEMTIME {
    convert(gmtime_r).unwrap_or_default()
}

/// The current local time.
pub fn local_time() -> SYSTEMTIME {
    unsafe { tzset() };
    convert(localtime_r).unwrap_or_else(system_time)
}

/// Fill `buf` with cryptographically secure random bytes. Pairing codes and
/// OAuth secrets depend on this, so it stops the server rather than go on
/// with predictable bytes.
pub fn random_fill(buf: &mut [u8]) {
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .expect("cannot read /dev/urandom");
}

/// `secrets.token_hex(n)`.
pub fn token_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    random_fill(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Windows needs a flag so child programs get no console window; nothing to do here.
pub fn no_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    cmd
}

/// Kill the process group `pid` leads (commands are started as group leaders)
/// and the process itself, in case it left the group.
pub fn kill_tree(pid: u32) {
    let Ok(pid) = i32::try_from(pid) else { return };
    if pid <= 1 {
        return;
    }
    unsafe {
        kill(-pid, SIGKILL);
        kill(pid, SIGKILL);
    }
}
