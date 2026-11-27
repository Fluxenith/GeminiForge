//! Python-compatibility helpers.
//!
//! The extension shows tool output and errors to Gemini verbatim, so the text
//! has to match what `server.py` produces: `str()` of JSON values, `int()` /
//! `float()` conversions and their error messages, code-point based slicing,
//! `str.splitlines()`, `json.dumps(..., ensure_ascii=False)` and so on.

use serde_json::{Map, Value};

pub const FAILSAFE_HTTP: &str = "The user moved the mouse to a screen corner; emergency stop.";
pub const FAILSAFE_STR: &str = "PyAutoGUI fail-safe triggered from mouse moving to a corner of the screen. To disable this fail-safe, set pyautogui.FAILSAFE to False. DISABLING FAIL-SAFE IS NOT RECOMMENDED.";

/// The exceptions the HTTP handler tells apart.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// `ToolError`: the message is sent as is.
    Tool(String),
    /// `TypeError`: sent as "Invalid arguments: ...".
    Type(String),
    /// `pyautogui.FailSafeException`.
    FailSafe,
    /// Any other exception: sent as "TypeName: message".
    Exc(String, String),
}

pub type Res<T> = Result<T, Error>;

impl Error {
    pub fn tool(msg: impl Into<String>) -> Error {
        Error::Tool(msg.into())
    }
    pub fn type_err(msg: impl Into<String>) -> Error {
        Error::Type(msg.into())
    }
    pub fn exc(kind: &str, msg: impl Into<String>) -> Error {
        Error::Exc(kind.to_string(), msg.into())
    }
    pub fn runtime(msg: impl Into<String>) -> Error {
        Error::Exc("RuntimeError".to_string(), msg.into())
    }
    pub fn value(msg: impl Into<String>) -> Error {
        Error::Exc("ValueError".to_string(), msg.into())
    }
    /// `str(exc)`
    pub fn py_str(&self) -> String {
        match self {
            Error::Tool(m) | Error::Type(m) | Error::Exc(_, m) => m.clone(),
            Error::FailSafe => FAILSAFE_STR.to_string(),
        }
    }
    /// The `error` field of the HTTP response.
    pub fn http_message(&self) -> String {
        match self {
            Error::Tool(m) => m.clone(),
            Error::Type(m) => format!("Invalid arguments: {m}"),
            Error::FailSafe => FAILSAFE_HTTP.to_string(),
            Error::Exc(kind, m) => format!("{kind}: {m}"),
        }
    }
    pub fn is_tool(&self) -> bool {
        matches!(self, Error::Tool(_))
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        os_error(&e, None)
    }
}

/// An `OSError` the way Python words it: `[WinError 5] text: 'path'`.
#[cfg(windows)]
pub fn os_error(e: &std::io::Error, path: Option<&str>) -> Error {
    match e.raw_os_error() {
        Some(code) => win_error(code as u32, path),
        None => Error::exc("OSError", e.to_string()),
    }
}

/// An `OSError` the way Python words it: `[Errno 2] text: 'path'`.
#[cfg(not(windows))]
pub fn os_error(e: &std::io::Error, path: Option<&str>) -> Error {
    let Some(code) = e.raw_os_error() else {
        return Error::exc("OSError", e.to_string());
    };
    let kind = match e.kind() {
        std::io::ErrorKind::NotFound => "FileNotFoundError",
        std::io::ErrorKind::PermissionDenied => "PermissionError",
        std::io::ErrorKind::AlreadyExists => "FileExistsError",
        std::io::ErrorKind::NotADirectory => "NotADirectoryError",
        std::io::ErrorKind::IsADirectory => "IsADirectoryError",
        _ => "OSError",
    };
    // the system's text without the " (os error N)" Rust adds
    let full = e.to_string();
    let text = full.strip_suffix(&format!(" (os error {code})")).unwrap_or(&full);
    let msg = match path {
        Some(p) => format!("[Errno {code}] {text}: {}", repr_str(p)),
        None => format!("[Errno {code}] {text}"),
    };
    Error::exc(kind, msg)
}

#[cfg(windows)]
pub fn win_error(code: u32, path: Option<&str>) -> Error {
    let kind = match code {
        2 | 3 | 15 | 18 | 53 | 67 | 161 => "FileNotFoundError",
        5 | 32 | 33 => "PermissionError",
        80 | 183 => "FileExistsError",
        267 => "NotADirectoryError",
        _ => "OSError",
    };
    let text = crate::ffi::win_error_text(code);
    let msg = match path {
        Some(p) => format!("[WinError {code}] {text}: {}", repr_str(p)),
        None => format!("[WinError {code}] {text}"),
    };
    Error::exc(kind, msg)
}

// ---------------------------------------------------------------- values

pub fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) => {
            if n.is_f64() {
                "float"
            } else {
                "int"
            }
        }
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// Python truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub fn opt_truthy(v: Option<&Value>, default: bool) -> bool {
    v.map(truthy).unwrap_or(default)
}

/// `repr(float)`
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let s = format!("{f:?}");
    match s.find('e') {
        None => s,
        Some(i) => {
            let (mant, exp) = (&s[..i], &s[i + 1..]);
            let (sign, digits) = match exp.strip_prefix('-') {
                Some(d) => ('-', d),
                None => ('+', exp),
            };
            let mant = mant.strip_suffix(".0").unwrap_or(mant);
            format!("{mant}e{sign}{digits:0>2}")
        }
    }
}

/// `repr(str)`
pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `repr(value)` for a value that came from `json.loads`.
pub fn repr(v: &Value) -> String {
    match v {
        Value::String(s) => repr_str(s),
        Value::Array(a) => format!("[{}]", a.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!(
            "{{{}}}",
            o.iter().map(|(k, v)| format!("{}: {}", repr_str(k), repr(v))).collect::<Vec<_>>().join(", ")
        ),
        other => py_str(other),
    }
}

/// `str(value)` for a value that came from `json.loads`.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                float_repr(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => s.clone(),
        other => repr(other),
    }
}

fn parse_int_literal(s: &str) -> Option<i64> {
    let t = strip(s);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let clean: String = digits.chars().filter(|c| *c != '_').collect();
    if clean.is_empty() || !clean.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v = clean.parse::<i64>().unwrap_or(i64::MAX);
    Some(if neg { v.wrapping_neg() } else { v })
}

/// `int(value)`
pub fn int(v: &Value) -> Res<i64> {
    match v {
        Value::Bool(b) => Ok(*b as i64),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i)
            } else if n.as_u64().is_some() {
                Ok(i64::MAX)
            } else {
                Ok(n.as_f64().unwrap_or(0.0).trunc() as i64)
            }
        }
        Value::String(s) => parse_int_literal(s)
            .ok_or_else(|| Error::value(format!("invalid literal for int() with base 10: {}", repr_str(s)))),
        other => Err(Error::type_err(format!(
            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
            type_name(other)
        ))),
    }
}

/// `float(value)`
pub fn float(v: &Value) -> Res<f64> {
    match v {
        Value::Bool(b) => Ok(*b as i64 as f64),
        Value::Number(n) => Ok(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => {
            let t = strip(s);
            let clean: String = t.chars().filter(|c| *c != '_').collect();
            let lower = clean.to_ascii_lowercase();
            let special = match lower.trim_start_matches(['+', '-']) {
                "inf" | "infinity" => Some(if lower.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY }),
                "nan" => Some(f64::NAN),
                _ => None,
            };
            if let Some(f) = special {
                return Ok(f);
            }
            let ok_chars = !clean.is_empty()
                && clean.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
            match clean.parse::<f64>() {
                Ok(f) if ok_chars => Ok(f),
                _ => Err(Error::value(format!("could not convert string to float: {}", repr_str(s)))),
            }
        }
        other => Err(Error::type_err(format!(
            "float() argument must be a string or a real number, not '{}'",
            type_name(other)
        ))),
    }
}

/// `round(x)` to an integer: halves go to the even neighbour.
pub fn round(x: f64) -> i64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 {
        let t = x.trunc();
        if (t as i64) % 2 == 0 {
            t as i64
        } else {
            r as i64
        }
    } else {
        r as i64
    }
}

// ---------------------------------------------------------------- strings

pub fn is_space(c: char) -> bool {
    matches!(c,
        '\t' | '\n' | '\x0b' | '\x0c' | '\r' | '\x1c'..='\x1f' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

/// `str.strip()`
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `len(str)`
pub fn len(s: &str) -> usize {
    s.chars().count()
}

fn byte_at(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

/// `s[:n]`
pub fn prefix(s: &str, n: usize) -> &str {
    &s[..byte_at(s, n)]
}

/// `s[-n:]` for n > 0
pub fn suffix(s: &str, n: usize) -> &str {
    let total = len(s);
    if n >= total {
        s
    } else {
        &s[byte_at(s, total - n)..]
    }
}

/// `s[start:end]`
pub fn slice(s: &str, start: usize, end: usize) -> &str {
    if end <= start {
        return "";
    }
    let a = byte_at(s, start);
    let b = byte_at(s, end);
    &s[a..b]
}

/// `str.splitlines()`
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let is_break = matches!(c,
            '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if !is_break {
            continue;
        }
        out.push(&s[start..i]);
        let mut next = i + c.len_utf8();
        if c == '\r' {
            if let Some(&(j, '\n')) = it.peek() {
                it.next();
                next = j + 1;
            }
        }
        start = next;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// `f"{n:,}"`
pub fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `re.sub(r"[\r\n]+", " ", text or "").strip()`
pub fn clean_one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_break = false;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            if !in_break {
                out.push(' ');
            }
            in_break = true;
        } else {
            in_break = false;
            out.push(c);
        }
    }
    strip(&out).to_string()
}

// ---------------------------------------------------------------- json

fn dump_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn dump_scalar(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_f64() {
                out.push_str(&float_repr(n.as_f64().unwrap_or(0.0)));
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::String(s) => dump_string(s, out),
        _ => {}
    }
}

fn dump_with(v: &Value, out: &mut String, item_sep: &str, key_sep: &str) {
    match v {
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                dump_with(item, out, item_sep, key_sep);
            }
            out.push(']');
        }
        Value::Object(o) => {
            out.push('{');
            for (i, (k, item)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                dump_string(k, out);
                out.push_str(key_sep);
                dump_with(item, out, item_sep, key_sep);
            }
            out.push('}');
        }
        scalar => dump_scalar(scalar, out),
    }
}

/// `json.dumps(value, ensure_ascii=False)`
pub fn json_dumps(v: &Value) -> String {
    let mut out = String::new();
    dump_with(v, &mut out, ", ", ": ");
    out
}

/// `json.dumps(value, ensure_ascii=False, separators=(",", ":"))`
pub fn json_compact(v: &Value) -> String {
    let mut out = String::new();
    dump_with(v, &mut out, ",", ":");
    out
}

fn dump_pretty(v: &Value, out: &mut String, indent: usize, level: usize) {
    let pad = |out: &mut String, n: usize| out.push_str(&" ".repeat(indent * n));
    match v {
        Value::Array(a) if !a.is_empty() => {
            out.push_str("[\n");
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                pad(out, level + 1);
                dump_pretty(item, out, indent, level + 1);
            }
            out.push('\n');
            pad(out, level);
            out.push(']');
        }
        Value::Object(o) if !o.is_empty() => {
            out.push_str("{\n");
            for (i, (k, item)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                pad(out, level + 1);
                dump_string(k, out);
                out.push_str(": ");
                dump_pretty(item, out, indent, level + 1);
            }
            out.push('\n');
            pad(out, level);
            out.push('}');
        }
        Value::Array(_) => out.push_str("[]"),
        Value::Object(_) => out.push_str("{}"),
        scalar => dump_scalar(scalar, out),
    }
}

/// `json.dumps(value, ensure_ascii=False, indent=indent)`
pub fn json_pretty(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    dump_pretty(v, &mut out, indent, 0);
    out
}

/// A value the way pydantic shows it in an error: `repr()`, shortened in the middle when long.
pub fn short_repr(v: &Value) -> String {
    let full = repr(v);
    let n = len(&full);
    if n <= 50 {
        return full;
    }
    format!("{}...{}", prefix(&full, 25), suffix(&full, 24))
}

/// `json.dumps(obj, ensure_ascii=False, indent=1)`
pub fn json_dumps_indent1(o: &Map<String, Value>) -> String {
    json_pretty(&Value::Object(o.clone()), 1)
}

// ---------------------------------------------------------------- keyword suggestions

fn substitution_cost(a: u8, b: u8) -> usize {
    if (a & 31) != (b & 31) {
        return 2;
    }
    if a == b {
        return 0;
    }
    if a.to_ascii_lowercase() == b.to_ascii_lowercase() {
        return 1;
    }
    2
}

fn levenshtein(a: &[u8], b: &[u8], max_cost: usize) -> usize {
    const MOVE: usize = 2;
    if a == b {
        return 0;
    }
    let (mut a, mut b) = (a, b);
    while !a.is_empty() && !b.is_empty() && a[0] == b[0] {
        a = &a[1..];
        b = &b[1..];
    }
    while !a.is_empty() && !b.is_empty() && a[a.len() - 1] == b[b.len() - 1] {
        a = &a[..a.len() - 1];
        b = &b[..b.len() - 1];
    }
    if a.is_empty() || b.is_empty() {
        return (a.len() + b.len()) * MOVE;
    }
    if a.len() > 40 || b.len() > 40 {
        return max_cost + 1;
    }
    if b.len() < a.len() {
        std::mem::swap(&mut a, &mut b);
    }
    if (b.len() - a.len()) * MOVE > max_cost {
        return max_cost + 1;
    }
    let mut buffer: Vec<usize> = (0..a.len()).map(|i| (i + 1) * MOVE).collect();
    let mut result = 0;
    for (b_index, &code) in b.iter().enumerate() {
        let mut left = b_index * MOVE;
        result = left;
        let mut minimum = usize::MAX;
        for (index, &ac) in a.iter().enumerate() {
            let substitute = left + substitution_cost(code, ac);
            left = buffer[index];
            let insert_delete = result.min(left) + MOVE;
            result = substitute.min(insert_delete);
            buffer[index] = result;
            minimum = minimum.min(result);
        }
        if minimum > max_cost {
            return max_cost + 1;
        }
    }
    result
}

/// CPython's "Did you mean ...?" pick for an unknown keyword argument.
pub fn suggest<'a>(candidates: &[&'a str], name: &str) -> Option<&'a str> {
    let mut best: Option<&str> = None;
    let mut best_distance = usize::MAX;
    for item in candidates {
        if *item == name {
            continue;
        }
        let mut max_distance = (name.len() + item.len() + 3) * 2 / 6;
        max_distance = max_distance.min(best_distance.wrapping_sub(1));
        let d = levenshtein(name.as_bytes(), item.as_bytes(), max_distance);
        if d > max_distance {
            continue;
        }
        if best.is_none() || d < best_distance {
            best = Some(item);
            best_distance = d;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn str_and_number_conversions_match_python() {
        assert_eq!(py_str(&json!(null)), "None");
        assert_eq!(py_str(&json!(true)), "True");
        assert_eq!(py_str(&json!(10)), "10");
        assert_eq!(py_str(&json!(10.5)), "10.5");
        assert_eq!(py_str(&json!(1.0)), "1.0");
        assert_eq!(py_str(&json!(1e16)), "1e+16");
        assert_eq!(py_str(&json!(1.5e-7)), "1.5e-07");
        assert_eq!(py_str(&json!(123456789.0)), "123456789.0");
        assert_eq!(py_str(&json!([1, "a", null, true, {"k": 1.0}])), "[1, 'a', None, True, {'k': 1.0}]");
        assert_eq!(int(&json!(" 7 ")).unwrap(), 7);
        assert_eq!(int(&json!("1_0")).unwrap(), 10);
        assert_eq!(int(&json!(1.9)).unwrap(), 1);
        assert_eq!(int(&json!(true)).unwrap(), 1);
        assert_eq!(
            int(&json!("abc")).unwrap_err().http_message(),
            "ValueError: invalid literal for int() with base 10: 'abc'"
        );
        assert_eq!(
            int(&json!(null)).unwrap_err().http_message(),
            "Invalid arguments: int() argument must be a string, a bytes-like object or a real number, not 'NoneType'"
        );
        assert_eq!(float(&json!("12")).unwrap(), 12.0);
        assert_eq!(
            float(&json!([1])).unwrap_err().http_message(),
            "Invalid arguments: float() argument must be a string or a real number, not 'list'"
        );
        assert_eq!(round(0.5), 0);
        assert_eq!(round(1.5), 2);
        assert_eq!(round(2.5), 2);
        assert_eq!(round(-0.5), 0);
        assert_eq!(round(2.6), 3);
    }

    #[test]
    fn string_helpers_count_code_points() {
        assert_eq!(len("日本ab"), 4);
        assert_eq!(prefix("日本ab", 3), "日本a");
        assert_eq!(suffix("日本ab", 3), "本ab");
        assert_eq!(slice("日本ab", 1, 3), "本a");
        assert_eq!(splitlines("a\r\nb\rc\n\nd\u{2028}e"), vec!["a", "b", "c", "", "d", "e"]);
        assert_eq!(splitlines("a\n"), vec!["a"]);
        assert!(splitlines("").is_empty());
        assert_eq!(commas(1234567), "1,234,567");
        assert_eq!(commas(12), "12");
        assert_eq!(strip("\u{3000} a \x1f"), "a");
        assert_eq!(clean_one_line(" a\r\n\nb "), "a b");
    }

    #[test]
    fn json_dumps_uses_python_separators_and_escapes() {
        let v = json!({"ok": true, "output": "a\tb\u{1}\u{7f}é\"", "image": null, "n": 1.0});
        assert_eq!(json_dumps(&v), "{\"ok\": true, \"output\": \"a\\tb\\u0001\u{7f}é\\\"\", \"image\": null, \"n\": 1.0}");
        assert_eq!(json_dumps(&json!(["a", 1])), "[\"a\", 1]");
    }

    #[test]
    fn keyword_suggestions_follow_cpython() {
        assert_eq!(suggest(&["path", "offset", "limit"], "pth"), Some("path"));
        assert_eq!(suggest(&["path", "offset", "limit"], "offest"), Some("offset"));
        assert_eq!(suggest(&["path", "offset", "limit"], "zzz"), None);
        assert_eq!(suggest(&["path", "old", "new", "replace_all"], "bogus"), None);
    }
}
