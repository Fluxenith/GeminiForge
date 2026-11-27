//! Automatic backups for files GeminiForge writes or edits, with restore
//! (port of server/backups.py; both servers share the same `backups/` folder
//! and the same on-disk format).

use crate::ffi;
use crate::lang::t as tr;
use crate::py::{self, Error, Res};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// larger files are not copied
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
pub const KEEP_DAYS: u64 = 30;
pub const KEEP_COUNT: usize = 5000;

struct Shared {
    root: Option<PathBuf>,
    last_id: String,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { root: None, last_id: String::new() });

fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("GF_BACKUPS").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(root) = crate::lang::repo_root() {
        return root.join("backups");
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("backups")
}

/// The backups folder (`ROOT` in backups.py).
pub fn root() -> PathBuf {
    let mut s = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if s.root.is_none() {
        s.root = Some(default_root());
    }
    s.root.clone().unwrap()
}

/// Point the backups at another folder (tests).
pub fn set_root(path: &Path) {
    let mut s = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    s.root = Some(path.to_path_buf());
    s.last_id.clear();
}

/// `\d{8}-\d{6}-\d{3,}` matched against the whole string.
pub fn is_backup_id(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 19 || b[8] != b'-' || b[15] != b'-' {
        return false;
    }
    s.chars().enumerate().all(|(i, c)| if i == 8 || i == 15 { c == '-' } else { c.is_ascii_digit() })
}

fn strftime_id() -> String {
    let t = ffi::local_time();
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

fn strftime_human() -> String {
    let t = ffi::local_time();
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

fn new_id(root: &Path, last_id: &mut String) -> String {
    let base = strftime_id();
    let mut n = 1;
    let mut bid = format!("{base}-{n:03}");
    while bid.as_str() <= last_id.as_str() || root.join(&bid).exists() {
        n += 1;
        bid = format!("{base}-{n:03}");
    }
    *last_id = bid.clone();
    bid
}

/// Days-since-epoch to (year, month, day) in the proleptic Gregorian calendar.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Local date `days` days ago as YYYYMMDD.
fn cutoff_date(days: u64) -> String {
    let t = ffi::local_time();
    let today = days_from_civil(t.wYear as i64, t.wMonth as i64, t.wDay as i64);
    let (y, m, d) = civil_from_days(today - days as i64);
    format!("{y:04}{m:02}{d:02}")
}

pub fn prune() {
    prune_in(&root());
}

fn prune_in(root: &Path) {
    let Ok(read) = fs::read_dir(root) else { return };
    let mut dirs: Vec<(String, PathBuf)> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (e.path().is_dir() && is_backup_id(&name)).then(|| (name, e.path()))
        })
        .collect();
    dirs.sort_by(|a, b| b.0.to_lowercase().cmp(&a.0.to_lowercase()));
    let cutoff = cutoff_date(KEEP_DAYS);
    for (i, (name, path)) in dirs.iter().enumerate() {
        if i >= KEEP_COUNT || &name[..8] < cutoff.as_str() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// Back up `path` before it changes. Returns (backup_id, note).
pub fn snapshot(path: &Path, tool: &str) -> Res<(String, String)> {
    let root = root();
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    fs::create_dir_all(&root)?;
    let bid = new_id(&root, &mut shared.last_id);
    let d = root.join(&bid);
    fs::create_dir(&d)?;
    let existed = path.is_file();
    let mut copied = false;
    let mut meta = Map::new();
    meta.insert("id".into(), json!(bid));
    meta.insert("time".into(), json!(strftime_human()));
    meta.insert("tool".into(), json!(tool));
    meta.insert("path".into(), json!(path.to_string_lossy()));
    meta.insert("existed".into(), json!(existed));
    meta.insert("copied".into(), json!(false));
    if existed {
        let size = fs::metadata(path)?.len();
        meta.insert("size".into(), json!(size));
        if size <= MAX_FILE_BYTES {
            fs::copy(path, d.join("file"))?;
            copied = true;
            meta.insert("copied".into(), json!(true));
        }
    }
    fs::write(d.join("meta.json"), py::json_dumps_indent1(&meta))?;
    prune_in(&root);
    drop(shared);
    if existed && !copied {
        let mb = (MAX_FILE_BYTES / 1024 / 1024).to_string();
        return Ok((bid, tr("bkTooLarge", &[("mb", &mb)])));
    }
    let note = tr("bkSaved", &[("id", &bid)]);
    Ok((bid, note))
}

fn entries() -> Vec<Map<String, Value>> {
    let root = root();
    let Ok(read) = fs::read_dir(&root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    dirs.sort_by(|a, b| {
        b.to_string_lossy().to_lowercase().cmp(&a.to_string_lossy().to_lowercase())
    });
    let mut out = Vec::new();
    for d in dirs {
        let m = d.join("meta.json");
        if d.is_dir() && m.is_file() {
            if let Ok(text) = fs::read_to_string(&m) {
                if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(&text) {
                    out.push(o);
                }
            }
        }
    }
    out
}

fn field<'a>(e: &'a Map<String, Value>, key: &str) -> Res<&'a Value> {
    e.get(key).ok_or_else(|| Error::exc("KeyError", py::repr_str(key)))
}

pub fn list_backups(limit: i64, path: Option<&str>) -> Res<String> {
    let mut items = entries();
    if let Some(p) = path.filter(|p| !p.is_empty()) {
        let key = p.replace('\\', "/").to_lowercase();
        let mut kept = Vec::new();
        for e in items {
            let ep = py::py_str(field(&e, "path")?).replace('\\', "/").to_lowercase();
            if ep.contains(&key) {
                kept.push(e);
            }
        }
        items = kept;
    }
    items.truncate(limit.clamp(1, 200) as usize);
    if items.is_empty() {
        return Ok(tr("bkNone", &[]));
    }
    let mut lines = Vec::new();
    for e in &items {
        let state = if !py::truthy(field(e, "existed")?) {
            tr("bkWasMissing", &[])
        } else if py::truthy(field(e, "copied")?) {
            String::new()
        } else {
            tr("bkNotCopied", &[])
        };
        let line = format!(
            "{}  {}  {:<14} {}  {}",
            py::py_str(field(e, "id")?),
            py::py_str(field(e, "time")?),
            py::py_str(field(e, "tool")?),
            py::py_str(field(e, "path")?),
            state
        );
        lines.push(line.trim_end_matches(py::is_space).to_string());
    }
    Ok(format!("{}\n{}", tr("bkHeader", &[]), lines.join("\n")))
}

/// `Err(Error::Exc("ValueError", ..))` is what the tool turns into a ToolError.
pub fn restore(backup_id: &str) -> Res<String> {
    let backup_id = py::strip(backup_id);
    if !is_backup_id(backup_id) {
        return Err(Error::value(tr("bkBadId", &[("id", backup_id)])));
    }
    let d = root().join(backup_id);
    let m = d.join("meta.json");
    if !m.is_file() {
        return Err(Error::value(tr("bkNotFound", &[("id", backup_id)])));
    }
    let text = fs::read_to_string(&m)?;
    let meta = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(o)) => o,
        _ => return Err(Error::value("Expecting value: line 1 column 1 (char 0)")),
    };
    let path_text = py::py_str(field(&meta, "path")?);
    let p = PathBuf::from(&path_text);
    let existed = py::truthy(field(&meta, "existed")?);
    if existed && (!py::truthy(field(&meta, "copied")?) || !d.join("file").is_file()) {
        return Err(Error::value(tr("bkNoContent", &[("id", backup_id)])));
    }
    let (undo_id, _) = snapshot(&p, &format!("restore {backup_id}"))?;
    let msg = if existed {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(d.join("file"), &p)?;
        let time = py::py_str(field(&meta, "time")?);
        tr("bkRestored", &[("path", &path_text), ("time", &time)])
    } else if p.is_file() {
        fs::remove_file(&p)?;
        tr("bkDeleted", &[("path", &path_text)])
    } else {
        tr("bkNothing", &[("path", &path_text)])
    };
    Ok(tr("bkUndo", &[("msg", &msg), ("id", &undo_id)]))
}

/// Command line (also used by restore_rs.bat), like `python backups.py`:
///     backups              list recent backups
///     backups list [N]     list the last N backups
///     backups restore ID   restore one backup
/// Returns the process exit code.
pub fn cli(argv: &[String]) -> i32 {
    let cmd = argv.first().map(|s| s.as_str()).unwrap_or("list");
    let result = match (cmd, argv.get(1)) {
        ("list", n) => {
            let limit = match n {
                Some(text) => match py::int(&Value::String(text.clone())) {
                    Ok(v) => v,
                    Err(e) => {
                        println!("{}", e.py_str());
                        return 1;
                    }
                },
                None => 20,
            };
            list_backups(limit, None)
        }
        ("restore", Some(id)) => restore(id),
        _ => {
            println!("Usage:\n    backups              list recent backups\n    backups list [N]     list the last N backups\n    backups restore ID   restore one backup");
            return 2;
        }
    };
    match result {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(e) => {
            println!("{}", e.py_str());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_ids_and_dates() {
        assert!(is_backup_id("20261103-011819-001"));
        assert!(is_backup_id("20261103-011819-1234"));
        assert!(!is_backup_id("20261103-011819-01"));
        assert!(!is_backup_id("manual-notes"));
        assert!(!is_backup_id("20261103_011819-001"));
        assert_eq!(civil_from_days(days_from_civil(2026, 11, 26)), (2026, 11, 26));
        assert_eq!(civil_from_days(days_from_civil(2024, 3, 1) - 1), (2024, 2, 29));
    }
}
