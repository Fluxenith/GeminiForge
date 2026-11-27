//! File tools: cd, list_dir, read_file, write_file, edit_file, search,
//! find_files, list_backups, restore_backup.

use crate::backups;
#[cfg(windows)]
use crate::ffi;
use crate::py::{self, Error, Res};
use crate::state::{self, MAX_READ_BYTES, SKIP_DIRS};
use crate::tools::{Args, ToolOut};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

// ---------------------------------------------------------------- paths

fn env(name: &str) -> Option<String> {
    if name.is_empty() || name.contains('=') || name.contains('\0') {
        return None;
    }
    std::env::var(name).ok()
}

/// `os.path.expanduser` (POSIX rules: `~` is HOME, `~name` another user's home)
#[cfg(not(windows))]
pub fn expanduser(path: &str) -> String {
    if !path.starts_with('~') {
        return path.to_string();
    }
    let end = path.find('/').unwrap_or(path.len());
    let (user, rest) = (&path[1..end], &path[end..]);
    let home = if user.is_empty() {
        match env("HOME") {
            Some(h) => h,
            None => return path.to_string(),
        }
    } else {
        // the other user's home is looked up in /etc/passwd, as pwd.getpwnam would
        let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
        match passwd.lines().map(|l| l.split(':').collect::<Vec<_>>()).find(|f| f.len() > 5 && f[0] == user) {
            Some(f) => f[5].to_string(),
            None => return path.to_string(),
        }
    };
    let home = if home.len() > 1 { home.trim_end_matches('/') } else { home.as_str() };
    format!("{home}{rest}")
}

/// `os.path.expanduser` (Windows rules)
#[cfg(windows)]
pub fn expanduser(path: &str) -> String {
    if !path.starts_with('~') {
        return path.to_string();
    }
    let chars: Vec<char> = path.chars().collect();
    let mut i = 1;
    while i < chars.len() && chars[i] != '/' && chars[i] != '\\' {
        i += 1;
    }
    let userhome = match env("USERPROFILE").filter(|s| !s.is_empty()) {
        Some(h) => h,
        None => match (env("HOMEDRIVE"), env("HOMEPATH")) {
            (drive, Some(p)) => format!("{}{}", drive.unwrap_or_default(), p),
            _ => return path.to_string(),
        },
    };
    let mut home = userhome;
    if i != 1 {
        let target: String = chars[1..i].iter().collect();
        let current = env("USERNAME").unwrap_or_default();
        if target != current {
            let base = Path::new(&home).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if current != base {
                return path.to_string();
            }
            let parent = Path::new(&home).parent().map(|p| p.to_path_buf()).unwrap_or_default();
            home = parent.join(target).to_string_lossy().into_owned();
        }
    }
    let rest: String = chars[i..].iter().collect();
    format!("{home}{rest}")
}

/// `os.path.expandvars` (Windows rules: %VAR%, $VAR, ${VAR}, no expansion in single quotes)
pub fn expandvars(path: &str) -> String {
    if !path.contains('$') && !path.contains('%') {
        return path.to_string();
    }
    let mut p: Vec<char> = path.chars().collect();
    let mut res = String::new();
    let mut index = 0usize;
    let is_varchar = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    while index < p.len() {
        let c = p[index];
        if c == '\'' {
            p = p[index + 1..].to_vec();
            match p.iter().position(|x| *x == '\'') {
                Some(i) => {
                    res.push(c);
                    res.extend(p[..=i].iter());
                    index = i;
                }
                None => {
                    res.push(c);
                    res.extend(p.iter());
                    index = p.len().wrapping_sub(1);
                }
            }
        } else if c == '%' {
            if p.get(index + 1) == Some(&'%') {
                res.push(c);
                index += 1;
            } else {
                p = p[index + 1..].to_vec();
                match p.iter().position(|x| *x == '%') {
                    None => {
                        res.push('%');
                        res.extend(p.iter());
                        index = p.len().wrapping_sub(1);
                    }
                    Some(i) => {
                        let var: String = p[..i].iter().collect();
                        res.push_str(&env(&var).unwrap_or_else(|| format!("%{var}%")));
                        index = i;
                    }
                }
            }
        } else if c == '$' {
            if p.get(index + 1) == Some(&'$') {
                res.push(c);
                index += 1;
            } else if p.get(index + 1) == Some(&'{') {
                p = p[index + 2..].to_vec();
                match p.iter().position(|x| *x == '}') {
                    None => {
                        res.push_str("${");
                        res.extend(p.iter());
                        index = p.len().wrapping_sub(1);
                    }
                    Some(i) => {
                        let var: String = p[..i].iter().collect();
                        res.push_str(&env(&var).unwrap_or_else(|| format!("${{{var}}}")));
                        index = i;
                    }
                }
            } else {
                let mut var = String::new();
                index += 1;
                while index < p.len() && is_varchar(p[index]) {
                    var.push(p[index]);
                    index += 1;
                }
                res.push_str(&env(&var).unwrap_or_else(|| format!("${var}")));
                if index < p.len() {
                    index -= 1;
                }
            }
        } else {
            res.push(c);
        }
        index = index.wrapping_add(1);
    }
    res
}

fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest.to_string())
    } else {
        p
    }
}

/// `Path.resolve()`: absolute, symlinks resolved, true on-disk case for the part that exists.
pub fn real_path(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut existing = abs.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(c) = fs::canonicalize(&existing) {
            let mut out = strip_verbatim(c);
            for r in rest.iter().rev() {
                out.push(r);
            }
            return out;
        }
        match (existing.parent().map(|x| x.to_path_buf()), existing.file_name().map(|x| x.to_os_string())) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return abs,
        }
    }
}

/// `resolve(path)` of server.py: falsy means the current folder.
pub fn resolve(path: &Value) -> PathBuf {
    if !py::truthy(path) {
        return PathBuf::from(state::cwd());
    }
    let text = expandvars(&expanduser(&py::py_str(path)));
    let mut p = PathBuf::from(&text);
    if !p.is_absolute() {
        p = Path::new(&state::cwd()).join(p);
    }
    real_path(&p)
}

pub fn show(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------- text

/// Legacy code pages are only tried on Windows, where Big5 and GBK files are
/// still common; elsewhere text that is not UTF-8 is read with replacement characters.
#[cfg(not(windows))]
fn mb_decode(_codepage: u32, _data: &[u8]) -> Option<String> {
    None
}

#[cfg(windows)]
fn mb_decode(codepage: u32, data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return Some(String::new());
    }
    const MB_ERR_INVALID_CHARS: u32 = 8;
    unsafe {
        let n = ffi::MultiByteToWideChar(codepage, MB_ERR_INVALID_CHARS, data.as_ptr(), data.len() as i32, std::ptr::null_mut(), 0);
        if n <= 0 {
            return None;
        }
        let mut buf = vec![0u16; n as usize];
        let m = ffi::MultiByteToWideChar(codepage, MB_ERR_INVALID_CHARS, data.as_ptr(), data.len() as i32, buf.as_mut_ptr(), n);
        if m <= 0 {
            return None;
        }
        let text = String::from_utf16(&buf[..m as usize]).ok()?;
        // Windows fills the code page's unassigned ranges with private-use
        // characters (and 0x80 with U+0080 or the euro sign); Python's codecs
        // reject those bytes, which is what lets a GBK file fall through cp950.
        let foreign =
            |c: char| ('\u{e000}'..='\u{f8ff}').contains(&c) || c == '\u{80}' || (codepage == 936 && c == '€');
        if text.chars().any(foreign) {
            return None;
        }
        Some(text)
    }
}

/// utf-8, then cp950, then gbk, then utf-8 with replacement characters.
pub fn decode_text(data: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(data) {
        return s.to_string();
    }
    if let Some(s) = mb_decode(950, data) {
        return s;
    }
    if let Some(s) = mb_decode(936, data) {
        return s;
    }
    String::from_utf8_lossy(data).into_owned()
}

pub fn read_text(p: &Path) -> std::io::Result<String> {
    Ok(decode_text(&fs::read(p)?))
}

pub fn is_binary(p: &Path) -> bool {
    let Ok(mut f) = fs::File::open(p) else { return true };
    let mut buf = [0u8; 4096];
    let mut filled = 0;
    loop {
        match f.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => {
                filled += n;
                if filled == buf.len() {
                    break;
                }
            }
            Err(_) => return true,
        }
    }
    buf[..filled].contains(&0)
}

// ---------------------------------------------------------------- tools

/// read_files takes at most this many paths, edit_files this many edits.
const MAX_BATCH_FILES: usize = 20;
const MAX_BATCH_EDITS: usize = 50;

pub fn t_cd(a: &Args) -> Res<ToolOut> {
    let p = resolve(a.req("path"));
    if !p.is_dir() {
        return Err(Error::tool(format!("Folder not found: {}", show(&p))));
    }
    let text = show(&p);
    state::with(|s| s.cwd = text.clone());
    Ok(format!("Current folder: {text}").into())
}

struct Lister {
    lines: Vec<String>,
    count: usize,
    depth: i64,
    directories_only: bool,
}

impl Lister {
    fn walk(&mut self, d: &Path, level: i64, prefix: &str) {
        let Ok(read) = fs::read_dir(d) else {
            self.lines.push(format!("{prefix}[access denied]"));
            return;
        };
        let mut entries: Vec<(bool, String, PathBuf)> = Vec::new();
        for e in read {
            let Ok(e) = e else {
                self.lines.push(format!("{prefix}[access denied]"));
                return;
            };
            let path = e.path();
            let is_dir = path.is_dir();
            if self.directories_only && !is_dir {
                continue;
            }
            entries.push((is_dir, e.file_name().to_string_lossy().into_owned(), path));
        }
        entries.sort_by(|a, b| (!a.0, a.1.to_lowercase()).cmp(&(!b.0, b.1.to_lowercase())));
        for (is_dir, name, path) in entries {
            self.count += 1;
            if self.count > 500 {
                self.lines.push(format!("{prefix}... (too many entries, rest omitted)"));
                return;
            }
            if is_dir {
                self.lines.push(format!("{prefix}{name}/"));
                if level < self.depth && !SKIP_DIRS.contains(&name.as_str()) {
                    self.walk(&path, level + 1, &format!("{prefix}  "));
                }
            } else {
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.lines.push(format!("{prefix}{name}  ({} B)", py::commas(size)));
            }
        }
    }
}

pub fn t_list_dir(a: &Args) -> Res<ToolOut> {
    let root = resolve(&a.or("path", json!(".")));
    if !root.is_dir() {
        return Err(Error::tool(format!("Folder not found: {}", show(&root))));
    }
    let depth = py::int(&a.or("depth", json!(1)))?.clamp(1, 4);
    let mut lister = Lister {
        lines: vec![show(&root)],
        count: 0,
        depth,
        directories_only: a.truthy("directories_only", false),
    };
    lister.walk(&root, 1, "  ");
    Ok(lister.lines.join("\n").into())
}

fn numbered(chunk: &[&str], offset: i64) -> String {
    chunk
        .iter()
        .enumerate()
        .map(|(k, l)| format!("{:>5}\t{}", offset + k as i64, l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lines of a file read as a stream (universal newlines), for files over MAX_READ_BYTES.
fn stream_lines(p: &Path, offset: i64, limit: i64) -> std::io::Result<(Vec<String>, bool)> {
    let mut reader = std::io::BufReader::with_capacity(1 << 16, fs::File::open(p)?);
    let mut chunk: Vec<String> = Vec::new();
    let mut has_more = false;
    let mut line: Vec<u8> = Vec::new();
    let mut i: i64 = 0;
    let mut pending_cr = false;
    let mut buf = [0u8; 1 << 16];
    let mut done = false;
    let emit = |line: &mut Vec<u8>, i: &mut i64, chunk: &mut Vec<String>, has_more: &mut bool| -> bool {
        *i += 1;
        let keep = *i >= offset;
        if keep && *i >= offset + limit {
            *has_more = true;
            return true;
        }
        if keep {
            chunk.push(String::from_utf8_lossy(line).into_owned());
        }
        line.clear();
        false
    };
    while !done {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if pending_cr {
                pending_cr = false;
                if b == b'\n' {
                    continue;
                }
            }
            if b == b'\n' || b == b'\r' {
                pending_cr = b == b'\r';
                if emit(&mut line, &mut i, &mut chunk, &mut has_more) {
                    done = true;
                    break;
                }
            } else {
                line.push(b);
            }
        }
    }
    if !done && !line.is_empty() {
        emit(&mut line, &mut i, &mut chunk, &mut has_more);
    }
    Ok((chunk, has_more))
}

pub fn t_read_file(a: &Args) -> Res<ToolOut> {
    let p = resolve(a.req("path"));
    if !p.is_file() {
        return Err(Error::tool(format!("File not found: {}", show(&p))));
    }
    if is_binary(&p) {
        return Err(Error::tool(format!("Binary file, cannot read as text: {}", show(&p))));
    }
    let offset = py::int(&a.or("offset", json!(1)))?.max(1);
    let limit = py::int(&a.or("limit", json!(400)))?.clamp(1, 2000);
    let size = fs::metadata(&p)?.len();
    if size > MAX_READ_BYTES.load(Ordering::Relaxed) {
        let (chunk, has_more) = stream_lines(&p, offset, limit)?;
        let refs: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
        let body = numbered(&refs, offset);
        let more = if has_more {
            format!(
                "\n... (large file {} B, streamed partial read; continue with offset={})",
                py::commas(size),
                offset + limit
            )
        } else {
            String::new()
        };
        return Ok(format!("{} (large file {} B)\n{}{}", show(&p), py::commas(size), body, more).into());
    }
    let text = read_text(&p)?;
    let lines = py::splitlines(&text);
    let start = ((offset - 1) as usize).min(lines.len());
    let end = (start + limit as usize).min(lines.len());
    let body = numbered(&lines[start..end], offset);
    let more = if (offset - 1 + limit) < lines.len() as i64 {
        format!("\n... ({} lines total; continue with offset={})", lines.len(), offset + limit)
    } else {
        String::new()
    };
    Ok(format!("{} ({} lines)\n{}{}", show(&p), lines.len(), body, more).into())
}

pub fn t_write_file(a: &Args) -> Res<ToolOut> {
    let p = resolve(a.req("path"));
    let content = a.req("content");
    if p.is_dir() {
        return Err(Error::tool(format!("This is a folder, not a file: {}", show(&p))));
    }
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let (_, note) = backups::snapshot(&p, "write_file")?;
    let Value::String(content) = content else {
        return Err(Error::type_err(format!("data must be str, not {}", py::type_name(content))));
    };
    fs::write(&p, content.as_bytes()).map_err(|e| py::os_error(&e, Some(&show(&p))))?;
    Ok(format!(
        "Wrote {} ({} characters, {} lines){}",
        show(&p),
        py::len(content),
        content.matches('\n').count() + 1,
        note
    )
    .into())
}

pub fn t_edit_file(a: &Args) -> Res<ToolOut> {
    let p = resolve(a.req("path"));
    let (old_v, new_v) = (a.req("old"), a.req("new"));
    let replace_all = a.truthy("replace_all", false);
    if !p.is_file() {
        return Err(Error::tool(format!("File not found: {}", show(&p))));
    }
    if is_binary(&p) {
        return Err(Error::tool(format!("Binary file, cannot edit as text: {}", show(&p))));
    }
    if !py::truthy(old_v) {
        return Err(Error::tool("`old` must not be empty."));
    }
    let text = read_text(&p)?;
    let (text, count) = replace_in(&text, old_v, new_v, replace_all)?;
    let (_, note) = backups::snapshot(&p, "edit_file")?;
    fs::write(&p, text.as_bytes()).map_err(|e| py::os_error(&e, Some(&show(&p))))?;
    Ok(format!("Edited {} ({count} replacement(s)){}", show(&p), note).into())
}

/// The replacement edit_file makes: `old` must be found exactly (a CRLF file
/// also matches LF text) and only once unless `replace_all`. Returns the new
/// text and the number of replacements.
fn replace_in(text: &str, old_v: &Value, new_v: &Value, replace_all: bool) -> Res<(String, usize)> {
    let Value::String(old) = old_v else {
        return Err(Error::type_err(format!("count() argument 1 must be str, not {}", py::type_name(old_v))));
    };
    let mut old = old.clone();
    let mut new: Option<String> = new_v.as_str().map(|s| s.to_string());
    let mut n = text.matches(old.as_str()).count();
    if n == 0 && text.contains("\r\n") {
        // tolerate CRLF files when Gemini sends LF (or mixed CRLF/LF) without doubling \r
        let old_crlf = old.replace("\r\n", "\n").replace('\n', "\r\n");
        if text.contains(old_crlf.as_str()) {
            old = old_crlf;
            let Some(new_s) = new.as_ref() else {
                return Err(Error::exc(
                    "AttributeError",
                    format!("'{}' object has no attribute 'replace'", py::type_name(new_v)),
                ));
            };
            new = Some(new_s.replace("\r\n", "\n").replace('\n', "\r\n"));
            n = text.matches(old.as_str()).count();
        }
    }
    if n == 0 {
        return Err(Error::tool("Text `old` not found (it must match the file exactly, including spaces and indentation). Check with read_file first."));
    }
    if n > 1 && !replace_all {
        return Err(Error::tool(format!(
            "`old` appears {n} times; add more context to make it unique, or set replace_all=true."
        )));
    }
    let Some(new) = new else {
        return Err(Error::type_err(format!("replace() argument 2 must be str, not {}", py::type_name(new_v))));
    };
    let text = if replace_all { text.replace(old.as_str(), &new) } else { text.replacen(old.as_str(), &new, 1) };
    Ok((text, if replace_all { n } else { 1 }))
}

/// read_files: up to 20 files in one call, each shown as read_file shows it.
/// A file that cannot be read gets its error in place of its content.
pub fn t_read_files(a: &Args) -> Res<ToolOut> {
    let paths = match a.req("paths") {
        Value::Array(items) => items.clone(),
        Value::String(one) => vec![Value::String(one.clone())],
        other => {
            return Err(Error::type_err(format!("paths must be a list of paths, not {}", py::type_name(other))));
        }
    };
    if paths.is_empty() {
        return Err(Error::tool("Give at least one path in paths."));
    }
    if paths.len() > MAX_BATCH_FILES {
        return Err(Error::tool(format!("At most {MAX_BATCH_FILES} files per read_files call.")));
    }
    let limit = a.or("limit", json!(200));
    py::int(&limit)?;
    let parts: Vec<String> = paths
        .iter()
        .map(|path| {
            let mut one = serde_json::Map::new();
            one.insert("path".to_string(), path.clone());
            one.insert("limit".to_string(), limit.clone());
            match t_read_file(&Args::new(&one)) {
                Ok(out) => out.text,
                Err(e) => format!("[{}] {}", py::py_str(path), e.http_message()),
            }
        })
        .collect();
    Ok(parts.join("\n\n").into())
}

/// edit_files: replacements in one or more files, checked together first.
/// If any edit does not apply, no file is changed. Each changed file is
/// backed up once and written once.
pub fn t_edit_files(a: &Args) -> Res<ToolOut> {
    let Value::Array(edits) = a.req("edits") else {
        return Err(Error::type_err(format!(
            "edits must be a list of {{path, old, new}} objects, not {}",
            py::type_name(a.req("edits"))
        )));
    };
    if edits.is_empty() {
        return Err(Error::tool("Give at least one edit in edits."));
    }
    if edits.len() > MAX_BATCH_EDITS {
        return Err(Error::tool(format!("At most {MAX_BATCH_EDITS} edits per edit_files call.")));
    }
    // (file, its text with the edits so far, replacements)
    let mut files: Vec<(PathBuf, String, usize)> = Vec::new();
    for (i, edit) in edits.iter().enumerate() {
        let fail = |msg: String| Error::tool(format!("Edit {} failed, so no file was changed: {msg}", i + 1));
        let Some(fields) = edit.as_object() else {
            return Err(fail("each edit is an object {\"path\", \"old\", \"new\", \"replace_all\"?}.".to_string()));
        };
        if let Some(key) = fields.keys().find(|k| !["path", "old", "new", "replace_all"].contains(&k.as_str())) {
            return Err(fail(format!("unknown key '{key}' (use path, old, new, replace_all).")));
        }
        let field = |k: &str| fields.get(k).unwrap_or(&Value::Null);
        if !py::truthy(field("path")) {
            return Err(fail("`path` is missing.".to_string()));
        }
        if !fields.contains_key("new") {
            return Err(fail("`new` is missing.".to_string()));
        }
        if !py::truthy(field("old")) {
            return Err(fail("`old` must not be empty.".to_string()));
        }
        let p = resolve(field("path"));
        let at = match files.iter().position(|(q, _, _)| *q == p) {
            Some(at) => at,
            None => {
                if !p.is_file() {
                    return Err(fail(format!("File not found: {}", show(&p))));
                }
                if is_binary(&p) {
                    return Err(fail(format!("Binary file, cannot edit as text: {}", show(&p))));
                }
                let text = read_text(&p).map_err(|e| fail(py::os_error(&e, Some(&show(&p))).http_message()))?;
                files.push((p, text, 0));
                files.len() - 1
            }
        };
        let replace_all = fields.get("replace_all").map(py::truthy).unwrap_or(false);
        let (text, n) =
            replace_in(&files[at].1, field("old"), field("new"), replace_all).map_err(|e| fail(e.http_message()))?;
        files[at].1 = text;
        files[at].2 += n;
    }
    let mut done = Vec::new();
    for (p, text, n) in &files {
        let (_, note) = backups::snapshot(p, "edit_files")?;
        fs::write(p, text.as_bytes()).map_err(|e| py::os_error(&e, Some(&show(p))))?;
        done.push(format!("Edited {} ({n} replacement(s)){note}", show(p)));
    }
    Ok(done.join("\n").into())
}

pub fn t_list_backups(a: &Args) -> Res<ToolOut> {
    let limit = py::int(&a.or("limit", json!(20)))?;
    let path = a.or("path", Value::Null);
    let key = if py::truthy(&path) { Some(show(&resolve(&path))) } else { None };
    Ok(backups::list_backups(limit, key.as_deref())?.into())
}

pub fn t_restore_backup(a: &Args) -> Res<ToolOut> {
    match backups::restore(&py::py_str(a.req("backup_id"))) {
        Ok(text) => Ok(text.into()),
        Err(Error::Exc(kind, msg)) if kind == "ValueError" => Err(Error::Tool(msg)),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------- matching

#[derive(Debug)]
enum Tok {
    Star,
    One,
    Lit(char),
    Class { negate: bool, items: Vec<(char, char)> },
}

/// `fnmatch.translate` tokens.
fn glob_tokens(pat: &[char]) -> Vec<Tok> {
    let mut out = Vec::new();
    let n = pat.len();
    let mut i = 0;
    while i < n {
        let c = pat[i];
        i += 1;
        match c {
            '*' => {
                if !matches!(out.last(), Some(Tok::Star)) {
                    out.push(Tok::Star);
                }
            }
            '?' => out.push(Tok::One),
            '[' => {
                let mut j = i;
                if j < n && pat[j] == '!' {
                    j += 1;
                }
                if j < n && pat[j] == ']' {
                    j += 1;
                }
                while j < n && pat[j] != ']' {
                    j += 1;
                }
                if j >= n {
                    out.push(Tok::Lit('['));
                } else {
                    let mut stuff = &pat[i..j];
                    let negate = stuff.first() == Some(&'!');
                    if negate {
                        stuff = &stuff[1..];
                    }
                    let mut items = Vec::new();
                    let mut k = 0;
                    while k < stuff.len() {
                        if k + 2 < stuff.len() && stuff[k + 1] == '-' {
                            if stuff[k] <= stuff[k + 2] {
                                items.push((stuff[k], stuff[k + 2]));
                            }
                            k += 3;
                        } else {
                            items.push((stuff[k], stuff[k]));
                            k += 1;
                        }
                    }
                    out.push(Tok::Class { negate, items });
                    i = j + 1;
                }
            }
            c => out.push(Tok::Lit(c)),
        }
    }
    out
}

fn match_tokens(toks: &[Tok], text: &[char]) -> bool {
    match toks.first() {
        None => text.is_empty(),
        Some(Tok::Star) => (0..=text.len()).any(|k| match_tokens(&toks[1..], &text[k..])),
        Some(tok) => {
            let Some(&c) = text.first() else { return false };
            let ok = match tok {
                Tok::One => true,
                Tok::Lit(l) => *l == c,
                Tok::Class { negate, items } => items.iter().any(|(lo, hi)| *lo <= c && c <= *hi) != *negate,
                Tok::Star => unreachable!(),
            };
            ok && match_tokens(&toks[1..], &text[1..])
        }
    }
}

/// `fnmatch.fnmatchcase(name, pat)`
pub fn fnmatch(name: &str, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = name.chars().collect();
    match_tokens(&glob_tokens(&p), &t)
}

fn match_segments(path: &[&str], pat: &[&str]) -> bool {
    match pat.first() {
        None => path.is_empty(),
        Some(&"**") => {
            if pat.len() == 1 {
                // a trailing ** needs something to match
                return !path.is_empty();
            }
            (0..=path.len()).any(|k| match_segments(&path[k..], &pat[1..]))
        }
        Some(seg) => match path.first() {
            Some(first) => fnmatch(first, seg) && match_segments(&path[1..], &pat[1..]),
            None => false,
        },
    }
}

/// `PurePosixPath(path).full_match(pattern)` (Python 3.13)
pub fn full_match(path: &str, pattern: &str) -> bool {
    if path.starts_with('/') != pattern.starts_with('/') {
        return false;
    }
    let split = |s: &'_ str| -> Vec<String> {
        s.split('/').filter(|x| !x.is_empty() && *x != ".").map(|x| x.to_string()).collect()
    };
    let (ps, gs) = (split(path), split(pattern));
    if gs.is_empty() {
        return false;
    }
    let pr: Vec<&str> = ps.iter().map(|s| s.as_str()).collect();
    let gr: Vec<&str> = gs.iter().map(|s| s.as_str()).collect();
    match_segments(&pr, &gr)
}

/// One segment of a compiled path pattern.
enum Seg {
    /// `**`
    Any,
    Toks(Vec<Tok>),
}

fn match_compiled(path: &[&str], pat: &[Seg]) -> bool {
    match pat.first() {
        None => path.is_empty(),
        Some(Seg::Any) => {
            if pat.len() == 1 {
                // a trailing ** needs something to match
                return !path.is_empty();
            }
            (0..=path.len()).any(|k| match_compiled(&path[k..], &pat[1..]))
        }
        Some(Seg::Toks(toks)) => match path.first() {
            Some(first) => {
                let chars: Vec<char> = first.chars().collect();
                match_tokens(toks, &chars) && match_compiled(&path[1..], &pat[1..])
            }
            None => false,
        },
    }
}

/// `_match_rel` with the pattern prepared once for a whole walk.
pub struct RelMatcher {
    /// a pattern without "/" is matched against the file name alone
    name: Option<Vec<Tok>>,
    absolute: bool,
    segs: Vec<Seg>,
}

impl RelMatcher {
    pub fn new(pattern: &Value) -> RelMatcher {
        let raw = if py::truthy(pattern) { py::py_str(pattern) } else { "*".to_string() };
        let mut pat = raw.replace('\\', "/");
        while let Some(rest) = pat.strip_prefix("./") {
            pat = rest.to_string();
        }
        let pat_l = pat.to_lowercase();
        if !pat_l.contains('/') {
            let chars: Vec<char> = pat_l.chars().collect();
            return RelMatcher { name: Some(glob_tokens(&chars)), absolute: false, segs: Vec::new() };
        }
        let segs = pat_l
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .map(|s| if s == "**" { Seg::Any } else { Seg::Toks(glob_tokens(&s.chars().collect::<Vec<char>>())) })
            .collect();
        RelMatcher { name: None, absolute: pat_l.starts_with('/'), segs }
    }

    /// Whether the relative path is needed (false: the file name is enough).
    pub fn uses_path(&self) -> bool {
        self.name.is_none()
    }

    pub fn matches(&self, rel_posix: &str, filename: &str) -> bool {
        if let Some(toks) = &self.name {
            // "*" matches every name: no need to look at it
            if matches!(toks.as_slice(), [Tok::Star]) {
                return true;
            }
            let chars: Vec<char> = filename.to_lowercase().chars().collect();
            return match_tokens(toks, &chars);
        }
        let rel_l = rel_posix.to_lowercase();
        if rel_l.starts_with('/') != self.absolute || self.segs.is_empty() {
            return false;
        }
        let parts: Vec<&str> = rel_l.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
        match_compiled(&parts, &self.segs)
    }
}

/// `_match_rel(rel_posix, filename, pattern)`
pub fn match_rel(rel_posix: &str, filename: &str, pattern: &Value) -> bool {
    RelMatcher::new(pattern).matches(rel_posix, filename)
}

/// Whether a link entry is a symbolic link (os.walk lists it but does not
/// enter it). Only Windows has junctions, which os.walk walks into.
#[cfg(not(windows))]
fn is_symbolic_link(_p: &Path) -> bool {
    true
}

/// Whether a link entry is a symbolic link (os.walk lists it but does not
/// enter it) rather than a junction (which os.walk walks into).
#[cfg(windows)]
fn is_symbolic_link(p: &Path) -> bool {
    const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
    let wide = ffi::wide(&p.to_string_lossy());
    unsafe {
        let mut data: ffi::WIN32_FIND_DATAW = std::mem::zeroed();
        let handle = ffi::FindFirstFileW(wide.as_ptr(), &mut data);
        if handle == -1 {
            return true;
        }
        ffi::FindClose(handle);
        data.dwFileAttributes & 0x400 != 0 && data.dwReserved0 == IO_REPARSE_TAG_SYMLINK
    }
}

/// `_walk_matches`: calls `visit` for every match in os.walk order; stops when it returns false.
fn walk_matches(root: &Path, pattern: &Value, include_dirs: bool, visit: &mut dyn FnMut(PathBuf) -> bool) -> bool {
    fn inner(
        root_rel: &str, cur: &Path, matcher: &RelMatcher, include_dirs: bool,
        visit: &mut dyn FnMut(PathBuf) -> bool,
    ) -> bool {
        let Ok(read) = fs::read_dir(cur) else { return true };
        // (name, do not enter)
        let mut dirs: Vec<(String, bool)> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        for e in read.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // The listing already says what each entry is; only links need a
            // second look (os.walk follows them to tell folders from files).
            let (is_dir, skip) = match e.file_type() {
                Ok(ft) if ft.is_symlink() => {
                    let path = e.path();
                    (path.is_dir(), is_symbolic_link(&path))
                }
                Ok(ft) => (ft.is_dir(), false),
                Err(_) => (false, false),
            };
            if is_dir {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    dirs.push((name, skip));
                }
            } else {
                files.push(name);
            }
        }
        dirs.sort();
        files.sort();
        // the path relative to the root is only built when the pattern looks at it
        let with_path = matcher.uses_path();
        let rel = |name: &str| -> String {
            if !with_path {
                String::new()
            } else if root_rel.is_empty() {
                name.to_string()
            } else {
                format!("{root_rel}/{name}")
            }
        };
        if include_dirs {
            for (d, _) in &dirs {
                if matcher.matches(&rel(d), d) && !visit(cur.join(d)) {
                    return false;
                }
            }
        }
        for f in &files {
            if matcher.matches(&rel(f), f) && !visit(cur.join(f)) {
                return false;
            }
        }
        for (d, skip) in &dirs {
            if *skip {
                continue;
            }
            if !inner(&rel(d), &cur.join(d), matcher, include_dirs, visit) {
                return false;
            }
        }
        true
    }
    inner("", root, &RelMatcher::new(pattern), include_dirs, visit)
}

/// The text of a file, or None for a binary or unreadable one (one open instead of two).
fn read_text_unless_binary(p: &Path) -> Option<String> {
    let mut f = fs::File::open(p).ok()?;
    let mut data = Vec::new();
    f.by_ref().take(4096).read_to_end(&mut data).ok()?;
    if data.contains(&0) {
        return None;
    }
    f.read_to_end(&mut data).ok()?;
    Some(decode_text(&data))
}

fn compile_regex(pattern: &Value, ignore_case: bool) -> Res<fancy_regex::Regex> {
    let Value::String(pat) = pattern else {
        return Err(Error::type_err("first argument must be string or compiled pattern"));
    };
    // Python spells "end of text" \Z
    let pat = pat.replace("\\Z", "\\z");
    let pat = if ignore_case { format!("(?i){pat}") } else { pat };
    fancy_regex::Regex::new(&pat).map_err(|e| Error::tool(format!("Invalid regular expression: {e}")))
}

pub fn t_search(a: &Args) -> Res<ToolOut> {
    let root = resolve(&a.or("path", json!(".")));
    let glob = a.or("glob", json!("*"));
    if !root.exists() {
        return Err(Error::tool(format!("Path not found: {}", show(&root))));
    }
    let rx = compile_regex(a.req("pattern"), a.truthy("ignore_case", true))?;
    let mut out: Vec<String> = Vec::new();
    let mut hits = 0usize;
    let mut full = false;
    let mut scan = |f: PathBuf| -> bool {
        let Ok(meta) = fs::metadata(&f) else { return true };
        if !meta.is_file() || meta.len() > 2_000_000 {
            return true;
        }
        let Some(text) = read_text_unless_binary(&f) else { return true };
        for (i, line) in py::splitlines(&text).iter().enumerate() {
            if rx.is_match(line).unwrap_or(false) {
                out.push(format!("{}:{}: {}", show(&f), i + 1, py::prefix(py::strip(line), 200)));
                hits += 1;
                if hits >= 200 {
                    out.push("... (over 200 matches; narrow the search)".to_string());
                    full = true;
                    return false;
                }
            }
        }
        true
    };
    if root.is_file() {
        scan(root.clone());
    } else {
        walk_matches(&root, &glob, false, &mut scan);
    }
    let _ = full;
    Ok(if out.is_empty() { "No matches found".to_string() } else { out.join("\n") }.into())
}

pub fn t_find_files(a: &Args) -> Res<ToolOut> {
    let root = resolve(&a.or("path", json!(".")));
    if !root.is_dir() {
        return Err(Error::tool(format!("Folder not found: {}", show(&root))));
    }
    let mut out: Vec<String> = Vec::new();
    walk_matches(&root, a.req("pattern"), true, &mut |f| {
        out.push(show(&f));
        if out.len() >= 300 {
            out.push("... (over 300 files)".to_string());
            return false;
        }
        true
    });
    Ok(if out.is_empty() { "No matching files found".to_string() } else { out.join("\n") }.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matching_follows_python() {
        assert!(fnmatch("app.py", "*.py"));
        assert!(!fnmatch("app.pyc", "*.py"));
        assert!(fnmatch("a1.txt", "a[0-9].t?t"));
        assert!(fnmatch("ab.txt", "a[!0-9]*"));
        assert!(fnmatch("a[b", "a[b"));
        for (path, pat, want) in [
            ("sub/mod.py", "sub/*.py", true),
            ("a/b/c.py", "**/*.py", true),
            ("c.py", "**/*.py", true),
            ("a", "a/**", false),
            ("a/b", "a/**", true),
            ("a/b/c", "a/*", false),
            ("a/b/c", "a/**/c", true),
            ("a/c", "a/**/c", true),
        ] {
            assert_eq!(full_match(path, pat), want, "{path} {pat}");
        }
        assert!(match_rel("Sub/Mod.PY", "Mod.PY", &json!(".\\sub\\*.py")));
        assert!(match_rel("x/y.txt", "y.txt", &Value::Null));
    }

    #[test]
    fn expandvars_handles_percent_and_dollar_forms() {
        std::env::set_var("GF_TEST_VAR", "VALUE");
        assert_eq!(expandvars("a%GF_TEST_VAR%b"), "aVALUEb");
        assert_eq!(expandvars("a$GF_TEST_VAR\\b"), "aVALUE\\b");
        assert_eq!(expandvars("a${GF_TEST_VAR}b"), "aVALUEb");
        assert_eq!(expandvars("%GF_NO_SUCH_VAR%"), "%GF_NO_SUCH_VAR%");
        assert_eq!(expandvars("100%"), "100%");
        assert_eq!(expandvars("'%GF_TEST_VAR%'"), "'%GF_TEST_VAR%'");
        assert_eq!(expandvars("plain"), "plain");
    }

    #[test]
    #[cfg(windows)]
    fn decode_text_falls_back_to_big5() {
        assert_eq!(decode_text("テスト".as_bytes()), "テスト");
        assert_eq!(decode_text(&[0xb4, 0xfa, 0xb8, 0xd5]), "測試");
        // "简体中文内容" in GBK is not valid cp950 for Python, so it must fall through to gbk
        let gbk = [0xbc, 0xf2, 0xcc, 0xe5, 0xd6, 0xd0, 0xce, 0xc4, 0xc4, 0xda, 0xc8, 0xdd];
        assert_eq!(decode_text(&gbk), "简体中文内容");
    }
}
