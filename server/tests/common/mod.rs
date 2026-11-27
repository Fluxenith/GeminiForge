//! Shared helpers for the integration tests.
//!
//! The server keeps process-wide state (current folder, jobs, remembered
//! elements), so every test takes `serial()` first. Nothing here sends live
//! desktop input, and files only go to temporary folders.
#![allow(dead_code)]

use geminiforge::py::Error;
use geminiforge::{fs_tools, tools};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn serial() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A temporary folder removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("gf_rs_test_{}_{}_{}", std::process::id(), tag, n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(fs_tools::real_path(&dir))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        for _ in 0..10 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

pub fn show(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `call_tool(name, args)` -> text
pub fn call(name: &str, args: Value) -> Result<String, Error> {
    tools::call_tool(&json!(name), args.as_object().cloned().unwrap_or_default()).map(|(text, _)| text)
}

pub fn call_full(name: &str, args: Value) -> Result<(String, Option<String>), Error> {
    tools::call_tool(&json!(name), args.as_object().cloned().unwrap_or_default())
}

/// The error text the extension would receive.
pub fn err(name: &str, args: Value) -> String {
    match call(name, args) {
        Ok(text) => panic!("expected {name} to fail, got: {text}"),
        Err(e) => e.http_message(),
    }
}

/// The first `YYYYMMDD-HHMMSS-NNN` backup id in a text.
pub fn backup_id(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len() {
        if chars.len() - start < 19 {
            break;
        }
        let mut end = start + 19;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
        let candidate: String = chars[start..end].iter().collect();
        if geminiforge::backups::is_backup_id(&candidate) {
            return candidate;
        }
    }
    panic!("no backup id in: {text}");
}
