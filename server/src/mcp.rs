//! GeminiForge MCP entry for Gemini's "@ custom app" (port of server/mcp_server.py).
//!
//! Gemini (gemini.google.com) calls the tools through an https URL. Locally this
//! listens on 127.0.0.1:18767; Tailscale Funnel publishes it.
//!
//! Security:
//! - Every MCP request needs an OAuth access token. Getting one requires the
//!   pairing code shown in this server's window (entered once when connecting).
//! - Tools that change things (write files, run commands, click...) pop up a
//!   confirmation window on this PC, unless approval mode says otherwise.
//!
//! Pairing and tokens live in `mcp_config.json` / `mcp_auth.json` next to it.

use crate::crypto;
use crate::ffi;
use crate::lang;
use crate::py::{self, Error};
use crate::state;
use crate::tools;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const DEFAULT_PORT: u16 = 18767;
const FUNNEL_PORT: u16 = 10000;
#[cfg(windows)]
const TAILSCALE: &str = r"C:\Program Files\Tailscale\tailscale.exe";
#[cfg(target_os = "macos")]
const TAILSCALE: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
#[cfg(all(unix, not(target_os = "macos")))]
const TAILSCALE: &str = "/usr/bin/tailscale";
const ACCESS_TTL: f64 = 24.0 * 3600.0;
const REFRESH_TTL: f64 = 180.0 * 24.0 * 3600.0;
pub static MAX_CLIENTS: AtomicUsize = AtomicUsize::new(100);
pub static MAX_MCP_BODY: AtomicUsize = AtomicUsize::new(32 * 1024 * 1024);
pub static MAX_LOG_BYTES: AtomicUsize = AtomicUsize::new(2 * 1024 * 1024);
/// bodies of the OAuth routes (forms and small JSON documents)
const MAX_AUTH_BODY: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 128;
const MAX_STREAMS: usize = 32;

pub const READ_ONLY: &[&str] = &[
    "cd", "list_dir", "read_file", "read_files", "search", "find_files", "screenshot", "wait", "wait_for",
    "job_status", "clipboard_get", "list_windows", "ui_elements", "read_window", "ocr_window", "zoom", "list_backups",
];

const PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const LATEST_PROTOCOL: &str = "2025-11-25";
const DENIED: &str = "The user denied this action on the computer. Stop; do not use other tools to get around the denial.";

fn log(msg: &str) {
    let t = ffi::local_time();
    println!("[{:02}:{:02}:{:02}] {msg}", t.wHour, t.wMinute, t.wSecond);
    let _ = std::io::stdout().flush();
}

fn stamp() -> String {
    let t = ffi::local_time();
    format!("{:02}-{:02} {:02}:{:02}:{:02}", t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

// ---------------------------------------------------------------- small helpers

/// `html.escape(text)`
pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

/// `urllib.parse.quote_plus(text)`
pub fn quote_plus(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(text: &str, plus_is_space: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match bytes[i] {
            b'+' if plus_is_space => out.push(b' '),
            b'%' => match (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
                (Some(a), Some(b)) => {
                    out.push((a * 16 + b) as u8);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `dict(parse_qsl(text, keep_blank_values=True))`: later keys win.
pub fn parse_form(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in text.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(k, true), percent_decode(v, true));
    }
    out
}

/// (scheme, netloc) of `urlparse(uri)`.
pub fn url_parts(uri: &str) -> (String, String) {
    let Some((scheme, rest)) = uri.split_once(':') else { return (String::new(), String::new()) };
    let valid = scheme.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !valid {
        return (String::new(), String::new());
    }
    let netloc = match rest.strip_prefix("//") {
        Some(after) => after.split(['/', '?', '#']).next().unwrap_or("").to_string(),
        None => String::new(),
    };
    (scheme.to_lowercase(), netloc)
}

fn atomic_write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_file_name(format!("tmp{}", ffi::token_hex(4)));
    std::fs::write(&tmp, text.as_bytes())?;
    let res = std::fs::rename(&tmp, path);
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

// ---------------------------------------------------------------- http messages

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn new(method: &str, target: &str) -> Request {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        Request { method: method.to_string(), path: path.to_string(), query: query.to_string(), headers: Vec::new(), body: Vec::new() }
    }
    pub fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).unwrap_or("")
    }
    pub fn with_header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
    pub fn with_body(mut self, body: &[u8]) -> Request {
        self.body = body.to_vec();
        self
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// an event stream that stays open (GET /mcp)
    pub stream: bool,
}

impl Response {
    fn new(status: u16, content_type: &str, body: Vec<u8>) -> Response {
        let mut headers = Vec::new();
        if !content_type.is_empty() {
            headers.push(("content-type".to_string(), content_type.to_string()));
        }
        Response { status, headers, body, stream: false }
    }
    /// Starlette's JSONResponse
    fn json(status: u16, value: Value) -> Response {
        Response::new(status, "application/json", py::json_compact(&value).into_bytes())
    }
    fn text(status: u16, text: &str) -> Response {
        Response::new(status, "text/plain; charset=utf-8", text.as_bytes().to_vec())
    }
    fn html(text: String) -> Response {
        Response::new(200, "text/html; charset=utf-8", text.into_bytes())
    }
    fn with(mut self, name: &str, value: &str) -> Response {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
    pub fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).unwrap_or("")
    }
    pub fn text_body(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

// ---------------------------------------------------------------- tool catalogue

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Str,
    Int,
    Num,
    Bool,
    StrList,
    DictList,
}

struct ParamSpec {
    name: String,
    kind: Kind,
    default: Option<Value>,
    required: bool,
}

pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
    params: Vec<ParamSpec>,
}

/// The tools as the Python server lists them (name, description, input schema).
pub fn catalogue() -> &'static [ToolSpec] {
    static CAT: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    CAT.get_or_init(|| {
        let raw: Value = serde_json::from_str(include_str!("mcp_tools.json")).expect("mcp_tools.json");
        raw.as_array()
            .expect("mcp_tools.json is a list")
            .iter()
            .map(|t| {
                let schema = t["inputSchema"].clone();
                let required: Vec<&str> =
                    schema["required"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
                let params = schema["properties"]
                    .as_object()
                    .map(|props| {
                        props
                            .iter()
                            .map(|(name, p)| {
                                let kind = match (p["type"].as_str().unwrap_or("string"), p["items"]["type"].as_str()) {
                                    ("integer", _) => Kind::Int,
                                    ("number", _) => Kind::Num,
                                    ("boolean", _) => Kind::Bool,
                                    ("array", Some("string")) => Kind::StrList,
                                    ("array", _) => Kind::DictList,
                                    _ => Kind::Str,
                                };
                                ParamSpec {
                                    name: name.clone(),
                                    kind,
                                    default: p.get("default").cloned(),
                                    required: required.contains(&name.as_str()),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                ToolSpec {
                    name: t["name"].as_str().unwrap_or("").to_string(),
                    description: t["description"].as_str().unwrap_or("").to_string(),
                    schema,
                    params,
                }
            })
            .collect()
    })
}

#[cfg(windows)]
pub const INSTRUCTIONS: &str = include_str!("mcp_instructions.txt");
/// macOS and Linux: the file and command tools only, for now.
#[cfg(not(windows))]
pub const INSTRUCTIONS: &str = include_str!("mcp_instructions_unix.txt");

/// A tool's description for this system.
fn description(t: &ToolSpec) -> String {
    if cfg!(windows) {
        t.description.clone()
    } else if t.name == "screenshot" {
        "Capture the screen to see what is on it (on macOS, monitor picks a display). Window capture, the grid and clicking are not available on this system yet.".to_string()
    } else {
        t.description.replace("Windows PowerShell", "bash")
    }
}

struct Invalid {
    field: String,
    msg: &'static str,
    kind: &'static str,
    input: Value,
}

fn coerce(kind: Kind, v: &Value) -> Result<Value, (&'static str, &'static str)> {
    let int_err = ("Input should be a valid integer", "int_type");
    let num_err = ("Input should be a valid number", "float_type");
    let bool_err = ("Input should be a valid boolean", "bool_type");
    match kind {
        Kind::Str => match v {
            Value::String(_) => Ok(v.clone()),
            _ => Err(("Input should be a valid string", "string_type")),
        },
        Kind::Int => match v {
            Value::Bool(b) => Ok(json!(*b as i64)),
            Value::Number(n) if n.is_f64() => {
                let f = n.as_f64().unwrap_or(0.0);
                if f.fract() == 0.0 && f.is_finite() {
                    Ok(json!(f as i64))
                } else {
                    Err(("Input should be a valid integer, got a number with a fractional part", "int_from_float"))
                }
            }
            Value::Number(_) => Ok(v.clone()),
            Value::String(s) => {
                let t = py::strip(s);
                if let Ok(i) = t.parse::<i64>() {
                    return Ok(json!(i));
                }
                match t.parse::<f64>() {
                    Ok(f) if f.fract() == 0.0 && f.is_finite() && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')) => {
                        Ok(json!(f as i64))
                    }
                    _ => Err(("Input should be a valid integer, unable to parse string as an integer", "int_parsing")),
                }
            }
            _ => Err(int_err),
        },
        Kind::Num => match v {
            Value::Bool(b) => Ok(json!(*b as i64 as f64)),
            Value::Number(n) => Ok(json!(n.as_f64().unwrap_or(0.0))),
            Value::String(s) => match py::strip(s).parse::<f64>() {
                Ok(f) => Ok(json!(f)),
                Err(_) => Err(("Input should be a valid number, unable to parse string as a number", "float_parsing")),
            },
            _ => Err(num_err),
        },
        Kind::Bool => match v {
            Value::Bool(_) => Ok(v.clone()),
            Value::Number(n) => match n.as_f64() {
                Some(f) if f == 0.0 => Ok(json!(false)),
                Some(f) if f == 1.0 => Ok(json!(true)),
                _ => Err(("Input should be a valid boolean, unable to interpret input", "bool_parsing")),
            },
            Value::String(s) => match s.to_lowercase().as_str() {
                "1" | "true" | "t" | "yes" | "y" | "on" => Ok(json!(true)),
                "0" | "false" | "f" | "no" | "n" | "off" => Ok(json!(false)),
                _ => Err(("Input should be a valid boolean, unable to interpret input", "bool_parsing")),
            },
            _ => Err(bool_err),
        },
        Kind::StrList | Kind::DictList => match v {
            Value::Array(_) => Ok(v.clone()),
            _ => Err(("Input should be a valid list", "list_type")),
        },
    }
}

fn py_type(v: &Value) -> &'static str {
    py::type_name(v)
}

/// What pydantic does with the arguments of a call: fill defaults, drop
/// omitted optional ones, convert types. `Err` is the validation error text.
pub fn validate(tool: &ToolSpec, args: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    let mut errors: Vec<Invalid> = Vec::new();
    for p in &tool.params {
        let optional = !p.required && p.default.is_none();
        match args.get(&p.name) {
            None => {
                if p.required {
                    errors.push(Invalid { field: p.name.clone(), msg: "Field required", kind: "missing", input: Value::Object(args.clone()) });
                } else if let Some(d) = &p.default {
                    out.insert(p.name.clone(), d.clone());
                }
            }
            Some(Value::Null) if optional => {}
            Some(v) => match coerce(p.kind, v) {
                Ok(Value::Array(items)) => {
                    let mut ok = true;
                    for (i, item) in items.iter().enumerate() {
                        let bad = match p.kind {
                            Kind::StrList if !item.is_string() => Some(("Input should be a valid string", "string_type")),
                            Kind::DictList if !item.is_object() => Some(("Input should be a valid dictionary", "dict_type")),
                            _ => None,
                        };
                        if let Some((msg, kind)) = bad {
                            ok = false;
                            errors.push(Invalid { field: format!("{}.{i}", p.name), msg, kind, input: item.clone() });
                        }
                    }
                    if ok {
                        out.insert(p.name.clone(), Value::Array(items));
                    }
                }
                Ok(value) => {
                    out.insert(p.name.clone(), value);
                }
                Err((msg, kind)) => errors.push(Invalid { field: p.name.clone(), msg, kind, input: v.clone() }),
            },
        }
    }
    if errors.is_empty() {
        return Ok(out);
    }
    let mut text = format!(
        "{} validation error{} for {}Arguments",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" },
        tool.name
    );
    for e in &errors {
        text.push_str(&format!(
            "\n{}\n  {} [type={}, input_value={}, input_type={}]\n    For further information visit https://errors.pydantic.dev/2.14/v/{}",
            e.field,
            e.msg,
            e.kind,
            py::short_repr(&e.input),
            py_type(&e.input),
            e.kind
        ));
    }
    Err(text)
}

// ---------------------------------------------------------------- the server

struct AuthCode {
    params: HashMap<String, String>,
    exp: f64,
}

/// Exit code of the confirmation popup: 0 = allow once, 2 = allow this tool for 30 minutes, anything else = deny.
pub type Approver = Box<dyn Fn(&str, &str) -> i32 + Send + Sync>;

pub struct Mcp {
    pub dir: PathBuf,
    pub port: u16,
    cfg: Mutex<Map<String, Value>>,
    store: Mutex<Map<String, Value>>,
    codes: Mutex<HashMap<String, AuthCode>>,
    failed_pins: Mutex<Vec<f64>>,
    allowed_until: Mutex<HashMap<String, f64>>,
    approval: Mutex<()>,
    approver: Approver,
    log_file: Mutex<()>,
    streams: AtomicUsize,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn detect_public_url() -> String {
    let exe = if Path::new(TAILSCALE).exists() { TAILSCALE } else { "tailscale" };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["status", "--json"]).stdin(std::process::Stdio::null());
    let out = ffi::no_window(&mut cmd).output();
    let Ok(out) = out else { return String::new() };
    let dns = serde_json::from_slice::<Value>(&out.stdout)
        .ok()
        .and_then(|v| v["Self"]["DNSName"].as_str().map(|s| s.trim_end_matches('.').to_string()))
        .unwrap_or_default();
    if dns.is_empty() {
        String::new()
    } else {
        format!("https://{dns}:{FUNNEL_PORT}")
    }
}

/// cloudflared on PATH, else where its installers put it.
fn cloudflared_program() -> Option<PathBuf> {
    let name = if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" };
    let on_path = std::env::var_os("PATH")
        .and_then(|paths| std::env::split_paths(&paths).map(|dir| dir.join(name)).find(|p| p.is_file()));
    let known: &[&str] = if cfg!(windows) {
        &[r"C:\Program Files (x86)\cloudflared\cloudflared.exe", r"C:\Program Files\cloudflared\cloudflared.exe"]
    } else {
        &["/opt/homebrew/bin/cloudflared", "/usr/local/bin/cloudflared", "/usr/bin/cloudflared"]
    };
    on_path.or_else(|| known.iter().map(PathBuf::from).find(|p| p.is_file()))
}

/// The quick tunnel address in a line of cloudflared's log, such as
/// `INF |  https://some-random-words.trycloudflare.com  |`.
pub fn quick_tunnel_url(line: &str) -> Option<String> {
    let mut rest = line;
    while let Some(at) = rest.find("https://") {
        let url: String = rest[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '.' | '-'))
            .collect();
        let host = &url["https://".len()..];
        // api.trycloudflare.com is where cloudflared asks for the tunnel, not the tunnel
        if host.ends_with(".trycloudflare.com") && host != "api.trycloudflare.com" && !host.contains('/') {
            return Some(url);
        }
        rest = &rest[at + "https://".len()..];
    }
    None
}

/// Start `cloudflared tunnel --url http://127.0.0.1:<port>` and wait for its
/// address. cloudflared shares this console (Windows) or process group, so
/// closing the window or Ctrl+C ends it together with the server.
fn start_quick_tunnel(port: u16) -> Result<String, String> {
    let Some(exe) = cloudflared_program() else {
        return Err(lang::t("mcpTunnelMissing", &[]));
    };
    let child = std::process::Command::new(exe)
        .args(["tunnel", "--no-autoupdate", "--url", &format!("http://{}:{port}", state::HOST)])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = child.map_err(|e| format!("cloudflared: {e}"))?;
    let Some(log) = child.stderr.take() else {
        return Err(lang::t("mcpTunnelFailed", &[]));
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // keep reading after the address so cloudflared never blocks on a full pipe
        let mut sent = false;
        for line in BufReader::new(log).lines().map_while(Result::ok) {
            if !sent {
                if let Some(url) = quick_tunnel_url(&line) {
                    sent = tx.send(url).is_ok();
                }
            }
        }
    });
    match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok(url) => Ok(url),
        Err(_) => {
            let _ = child.kill();
            Err(lang::t("mcpTunnelFailed", &[]))
        }
    }
}

/// The folder with mcp_config.json / mcp_auth.json: `GF_MCP_DIR`, else the
/// `server` folder of the GeminiForge checkout, else the folder of the executable.
pub fn default_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GF_MCP_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(root) = lang::repo_root() {
        let server = root.join("server");
        if server.is_dir() {
            return server;
        }
    }
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

fn spawn_approval(tool: &str, payload: &str) -> i32 {
    let Ok(exe) = std::env::current_exe() else { return 1 };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["approve", tool, "-", "45"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = ffi::no_window(&mut cmd).spawn();
    let Ok(mut child) = child else { return 1 };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }
    // Gemini gives up on a tool call after about a minute, so the popup auto-denies at 45 s.
    let deadline = state::now() + 60.0;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code().unwrap_or(1),
            Ok(None) if state::now() < deadline => state::sleep(0.05),
            _ => {
                let _ = child.kill();
                return 1;
            }
        }
    }
}

impl Mcp {
    pub fn new(dir: &Path, port: u16) -> Mcp {
        Mcp::with_approver(dir, port, Box::new(spawn_approval), true)
    }

    /// `detect` asks Tailscale for the public address when the config has none.
    pub fn with_approver(dir: &Path, port: u16, approver: Approver, detect: bool) -> Mcp {
        let config_file = dir.join("mcp_config.json");
        let mut cfg = std::fs::read_to_string(&config_file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let defaults: Vec<(&str, Value)> = vec![
            ("public_url", json!("")),
            ("pairing_code", json!(crypto::pairing_code())),
            ("client_id", json!("geminiforge")),
            ("client_secret", json!(crypto::token_urlsafe(32))),
            // readonly: confirm everything except read-only tools; all: confirm every tool; none: never confirm
            ("approval", json!("readonly")),
            // on: Gemini web asks before non-read-only tools; off: report every tool as read-only
            // so Gemini never asks (the user's choice -- injected instructions then run unchecked)
            ("web_confirm", json!("on")),
            // tailscale: public_url is the Tailscale Funnel address (found once and saved);
            // cloudflared: start a cloudflared quick tunnel, whose address changes at every start
            ("tunnel", json!("tailscale")),
        ];
        let mut changed = false;
        for (k, v) in defaults {
            if !cfg.contains_key(k) {
                cfg.insert(k.to_string(), v);
                changed = true;
            }
        }
        if detect && !py::truthy(&cfg["public_url"]) {
            let url = detect_public_url();
            changed = changed || !url.is_empty();
            cfg.insert("public_url".to_string(), json!(url));
        }
        if changed {
            let _ = atomic_write(&config_file, &py::json_pretty(&Value::Object(cfg.clone()), 2));
        }
        let mut store = std::fs::read_to_string(dir.join("mcp_auth.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        for key in ["clients", "tokens"] {
            if !store.get(key).map(|v| v.is_object()).unwrap_or(false) {
                store.insert(key.to_string(), json!({}));
            }
        }
        Mcp {
            dir: dir.to_path_buf(),
            port,
            cfg: Mutex::new(cfg),
            store: Mutex::new(store),
            codes: Mutex::new(HashMap::new()),
            failed_pins: Mutex::new(Vec::new()),
            allowed_until: Mutex::new(HashMap::new()),
            approval: Mutex::new(()),
            approver,
            log_file: Mutex::new(()),
            streams: AtomicUsize::new(0),
        }
    }

    pub fn cfg(&self, key: &str) -> String {
        lock(&self.cfg).get(key).map(py::py_str).unwrap_or_default()
    }

    pub fn set_cfg(&self, key: &str, value: &str) {
        lock(&self.cfg).insert(key.to_string(), json!(value));
    }

    fn public_url(&self, req: &Request) -> String {
        let url = self.cfg("public_url");
        if !url.is_empty() {
            return url.trim_end_matches('/').to_string();
        }
        let host = req.header("host");
        if host.is_empty() {
            format!("https://127.0.0.1:{}", self.port)
        } else {
            format!("https://{host}")
        }
    }

    // ------------------------------------------------------------ oauth store

    fn save_store(&self, store: &mut Map<String, Value>) {
        let now = state::now();
        if let Some(Value::Object(tokens)) = store.get_mut("tokens") {
            tokens.retain(|_, v| v["exp"].as_f64().unwrap_or(0.0) > now);
        }
        let _ = atomic_write(&self.dir.join("mcp_auth.json"), &py::json_pretty(&Value::Object(store.clone()), 1));
    }

    fn issue_tokens_in(&self, store: &mut Map<String, Value>, client_id: &str) -> Value {
        let (access, refresh) = (crypto::token_urlsafe(32), crypto::token_urlsafe(40));
        let now = state::now();
        if let Some(Value::Object(tokens)) = store.get_mut("tokens") {
            tokens.insert(
                crypto::sha256_hex(&access),
                obj(vec![("kind", json!("access")), ("client_id", json!(client_id)), ("exp", json!(now + ACCESS_TTL))]),
            );
            tokens.insert(
                crypto::sha256_hex(&refresh),
                obj(vec![("kind", json!("refresh")), ("client_id", json!(client_id)), ("exp", json!(now + REFRESH_TTL))]),
            );
        }
        self.save_store(store);
        obj(vec![
            ("access_token", json!(access)),
            ("token_type", json!("Bearer")),
            ("expires_in", json!(ACCESS_TTL as i64)),
            ("refresh_token", json!(refresh)),
            ("scope", json!("geminiforge")),
        ])
    }

    pub fn issue_tokens(&self, client_id: &str) -> Value {
        let mut store = lock(&self.store);
        self.issue_tokens_in(&mut store, client_id)
    }

    fn token_record(store: &Map<String, Value>, token: &str, kind: &str) -> Option<Value> {
        let rec = store.get("tokens")?.get(crypto::sha256_hex(token))?;
        (rec["kind"].as_str() == Some(kind) && rec["exp"].as_f64().unwrap_or(0.0) > state::now()).then(|| rec.clone())
    }

    pub fn token_valid(&self, token: &str, kind: &str) -> bool {
        Mcp::token_record(&lock(&self.store), token, kind).is_some()
    }

    pub fn client_count(&self) -> usize {
        lock(&self.store).get("clients").and_then(|c| c.as_object()).map(|c| c.len()).unwrap_or(0)
    }

    pub fn has_client(&self, client_id: &str) -> bool {
        lock(&self.store).get("clients").and_then(|c| c.get(client_id)).is_some()
    }

    fn get_client(&self, client_id: &str) -> Option<Value> {
        if client_id == self.cfg("client_id") {
            return Some(obj(vec![
                ("client_id", json!(client_id)),
                ("client_secret", json!(self.cfg("client_secret"))),
                ("redirect_uris", Value::Null),
                ("client_name", json!("Gemini")),
                ("auth", json!("client_secret")),
            ]));
        }
        lock(&self.store).get("clients").and_then(|c| c.get(client_id)).cloned()
    }

    fn redirect_ok(client: &Value, uri: &str) -> bool {
        let (scheme, netloc) = url_parts(uri);
        if scheme != "https" || netloc.is_empty() {
            return false;
        }
        match client["redirect_uris"].as_array() {
            Some(list) if !list.is_empty() => list.iter().any(|u| u.as_str() == Some(uri)),
            _ => true,
        }
    }

    fn authorized(&self, req: &Request) -> bool {
        let auth = req.header("authorization");
        let is_bearer = auth.get(..7).map(|p| p.eq_ignore_ascii_case("bearer ")).unwrap_or(false);
        is_bearer && self.token_valid(auth[7..].trim(), "access")
    }

    // ------------------------------------------------------------ oauth routes

    fn resource_metadata(&self, req: &Request) -> Response {
        let base = self.public_url(req);
        Response::json(
            200,
            obj(vec![
                ("resource", json!(format!("{base}/mcp"))),
                ("authorization_servers", json!([base])),
                ("bearer_methods_supported", json!(["header"])),
                ("scopes_supported", json!(["geminiforge"])),
            ]),
        )
    }

    fn server_metadata(&self, req: &Request) -> Response {
        let base = self.public_url(req);
        Response::json(
            200,
            obj(vec![
                ("issuer", json!(base)),
                ("authorization_endpoint", json!(format!("{base}/authorize"))),
                ("token_endpoint", json!(format!("{base}/token"))),
                ("registration_endpoint", json!(format!("{base}/register"))),
                ("response_types_supported", json!(["code"])),
                ("grant_types_supported", json!(["authorization_code", "refresh_token"])),
                ("code_challenge_methods_supported", json!(["S256"])),
                ("token_endpoint_auth_methods_supported", json!(["client_secret_post", "client_secret_basic", "none"])),
                ("scopes_supported", json!(["geminiforge"])),
            ]),
        )
    }

    fn register(&self, req: &Request) -> Response {
        let invalid = |what: &str| Response::json(400, obj(vec![("error", json!(what))]));
        let Some(Value::Object(body)) = serde_json::from_slice::<Value>(&req.body).ok() else {
            return invalid("invalid_client_metadata");
        };
        let Some(raw_uris) = body.get("redirect_uris").and_then(|v| v.as_array()) else {
            return invalid("invalid_redirect_uri");
        };
        let uris: Vec<String> = raw_uris
            .iter()
            .filter_map(|u| u.as_str())
            .filter(|u| {
                let (scheme, netloc) = url_parts(u);
                scheme == "https" && !netloc.is_empty()
            })
            .map(|u| u.to_string())
            .collect();
        if uris.is_empty() {
            return invalid("invalid_redirect_uri");
        }
        let method = match body.get("token_endpoint_auth_method") {
            Some(v) if py::truthy(v) => v.clone(),
            _ => json!("client_secret_post"),
        };
        let name = match body.get("client_name") {
            Some(v) if py::truthy(v) => py::py_str(v),
            _ => "MCP client".to_string(),
        };
        let client_name = py::prefix(&name, 80).to_string();
        let client_id = format!("gf-{}", crypto::token_urlsafe(12));
        let public = method.as_str() == Some("none");
        let now = state::now() as i64;
        let mut client = Map::new();
        client.insert("client_id".into(), json!(client_id));
        client.insert("redirect_uris".into(), json!(uris));
        client.insert("client_name".into(), json!(client_name));
        client.insert("auth".into(), json!(if public { "none" } else { "client_secret" }));
        client.insert("created_at".into(), json!(now));
        let secret = (!public).then(|| crypto::token_urlsafe(32));
        if let Some(s) = &secret {
            client.insert("client_secret".into(), json!(s));
        }
        {
            let mut store = lock(&self.store);
            let max = MAX_CLIENTS.load(Ordering::Relaxed);
            let t_now = state::now();
            let active: Vec<String> = store["tokens"]
                .as_object()
                .map(|t| {
                    t.values()
                        .filter(|v| v["exp"].as_f64().unwrap_or(0.0) > t_now)
                        .filter_map(|v| v["client_id"].as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let Some(Value::Object(clients)) = store.get_mut("clients") else { return invalid("invalid_client_metadata") };
            if clients.len() >= max {
                let mut removable: Vec<(String, i64)> = clients
                    .iter()
                    .filter(|(cid, _)| !active.contains(cid))
                    .map(|(cid, c)| (cid.clone(), c["created_at"].as_i64().unwrap_or(0)))
                    .collect();
                removable.sort_by_key(|(_, created)| *created);
                let need = clients.len() - max + 1;
                for (cid, _) in removable.into_iter().take(need) {
                    clients.shift_remove(&cid);
                }
                if clients.len() >= max {
                    return Response::json(429, obj(vec![("error", json!("too_many_clients"))]));
                }
            }
            clients.insert(client_id.clone(), Value::Object(client));
            self.save_store(&mut store);
        }
        let hosts: Vec<String> = uris.iter().map(|u| url_parts(u).1).collect();
        log(&lang::t("mcpClientRegistered", &[("name", &client_name), ("hosts", &hosts.join(", "))]));
        let mut out = vec![
            ("client_id", json!(client_id)),
            ("redirect_uris", json!(uris)),
            ("client_name", json!(client_name)),
            ("token_endpoint_auth_method", method),
            ("grant_types", json!(["authorization_code", "refresh_token"])),
            ("response_types", json!(["code"])),
            ("client_id_issued_at", json!(now)),
        ];
        if let Some(s) = secret {
            out.push(("client_secret", json!(s)));
            out.push(("client_secret_expires_at", json!(0)));
        }
        Response::json(201, obj(out))
    }

    fn page(&self, params: &[(&str, String)], client: &Value, language: &'static str, error_key: Option<&str>) -> Response {
        let m = |key: &str, values: &[(&str, &str)]| html_escape(&lang::t_in(language, key, values));
        let hidden: String = params
            .iter()
            .map(|(k, v)| format!("<input type=\"hidden\" name=\"{k}\" value=\"{}\">", html_escape(v)))
            .collect();
        let client_name = match &client["client_name"] {
            v if py::truthy(v) => py::py_str(v),
            _ => "Gemini".to_string(),
        };
        let redirect = params.iter().find(|(k, _)| *k == "redirect_uri").map(|(_, v)| v.clone()).unwrap_or_default();
        let dest = url_parts(&redirect).1;
        let err = error_key.map(|k| format!("<p class=\"e\">{}</p>", m(k, &[]))).unwrap_or_default();
        let html = format!(
            "<!doctype html><html lang=\"{language}\"><head><meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title>\n\
<style>body{{font:16px/1.6 \"Segoe UI\",system-ui,sans-serif;background:#131314;color:#e3e3e3;\n\
display:flex;justify-content:center;padding:48px 16px;margin:0}}.c{{max-width:420px;width:100%;background:#1e1f20;\n\
border:1px solid #3c4043;border-radius:16px;padding:28px}}h1{{font-size:20px;margin:0 0 8px}}p{{color:#bdc1c6}}\n\
input{{font:22px Consolas,monospace;letter-spacing:4px;width:100%;box-sizing:border-box;padding:10px 12px;\n\
border-radius:10px;border:1px solid #5f6368;background:#131314;color:#fff;margin:8px 0 16px}}\n\
button{{font:inherit;width:100%;padding:10px;border:0;border-radius:10px;background:#1a73e8;color:#fff;cursor:pointer}}\n\
.e{{color:#f28b82}}small{{color:#9aa0a6}}</style></head><body><div class=\"c\">\n\
<h1>⚒ {link}</h1>\n\
<p>{access}</p>\n\
<p><label for=\"pairing_code\">{pair}</label></p>\n\
{err}<form method=\"post\" action=\"authorize\">{hidden}\n\
<input id=\"pairing_code\" name=\"pairing_code\" inputmode=\"numeric\" autocomplete=\"off\" autofocus required>\n\
<button type=\"submit\">{allow}</button></form>\n\
<p><small>{redirect_to}</small></p></div></body></html>",
            title = m("authTitle", &[]),
            link = m("linkApp", &[]),
            access = m("accessRequest", &[("client", &client_name)]),
            pair = m("pairCode", &[]),
            allow = m("allowLink", &[]),
            redirect_to = m("redirectTo", &[("dest", &dest)]),
        );
        Response::html(html)
    }

    fn authorize(&self, req: &Request) -> Response {
        let language = lang::current_language();
        let msg = |key: &str| lang::t_in(language, key, &[]);
        let src = if req.method == "POST" { parse_form(&String::from_utf8_lossy(&req.body)) } else { parse_form(&req.query) };
        let keys = ["response_type", "client_id", "redirect_uri", "state", "code_challenge", "code_challenge_method", "scope", "resource"];
        let params: Vec<(&str, String)> = keys.iter().map(|k| (*k, src.get(*k).cloned().unwrap_or_default())).collect();
        let get = |k: &str| params.iter().find(|(key, _)| *key == k).map(|(_, v)| v.as_str()).unwrap_or("");
        let Some(client) = self.get_client(get("client_id")) else {
            return Response::text(400, &msg("unknownClient"));
        };
        if !Mcp::redirect_ok(&client, get("redirect_uri")) {
            return Response::text(400, &msg("badRedirect"));
        }
        if get("response_type") != "code" {
            return Response::text(400, &msg("onlyCode"));
        }
        if client["auth"].as_str() == Some("none") && get("code_challenge").is_empty() {
            return Response::text(400, &msg("needPkce"));
        }
        if !get("code_challenge").is_empty() && !matches!(get("code_challenge_method"), "" | "S256") {
            return Response::text(400, "Only S256 PKCE is supported.");
        }
        if req.method != "POST" {
            return self.page(&params, &client, language, None);
        }

        let now = state::now();
        {
            let mut failed = lock(&self.failed_pins);
            failed.retain(|t| now - *t < 600.0);
            if failed.len() >= 5 {
                return self.page(&params, &client, language, Some("tooMany"));
            }
            let given = src.get("pairing_code").map(|s| py::strip(s).to_string()).unwrap_or_default();
            if !crypto::constant_time_eq(&given, &self.cfg("pairing_code")) {
                failed.push(now);
                log(&format!("⚠️ {}", lang::t("mcpWrongCode", &[])));
                return self.page(&params, &client, language, Some("wrongCode"));
            }
        }
        let code = crypto::token_urlsafe(32);
        {
            let mut codes = lock(&self.codes);
            codes.retain(|_, v| v.exp > now);
            let stored = params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            codes.insert(code.clone(), AuthCode { params: stored, exp: now + 300.0 });
        }
        let client_name = match &client["client_name"] {
            Value::Null => "None".to_string(),
            v => py::py_str(v),
        };
        log(&format!("✅ {}", lang::t("mcpLinked", &[("name", &client_name)])));
        let mut q = vec![("code", code), ("iss", self.public_url(req))];
        if !get("state").is_empty() {
            q.push(("state", get("state").to_string()));
        }
        let query: Vec<String> = q.iter().map(|(k, v)| format!("{k}={}", quote_plus(v))).collect();
        let redirect = get("redirect_uri");
        let sep = if redirect.contains('?') { "&" } else { "?" };
        Response { status: 302, headers: vec![("location".to_string(), format!("{redirect}{sep}{}", query.join("&")))], body: Vec::new(), stream: false }
    }

    fn client_auth(&self, req: &Request, form: &HashMap<String, String>) -> Option<Value> {
        let mut cid = form.get("client_id").cloned();
        let mut secret = form.get("client_secret").cloned();
        let auth = req.header("authorization");
        if auth.get(..6).map(|p| p.eq_ignore_ascii_case("basic ")).unwrap_or(false) {
            let raw = String::from_utf8(crypto::base64_decode(&auth[6..])?).ok()?;
            let (a, b) = raw.split_once(':')?;
            cid = Some(percent_decode(a, false));
            secret = Some(percent_decode(b, false));
        }
        let client = self.get_client(&cid.unwrap_or_default())?;
        if client["auth"].as_str() == Some("none") {
            return Some(client);
        }
        let secret = secret.filter(|s| !s.is_empty())?;
        crypto::constant_time_eq(&secret, client["client_secret"].as_str().unwrap_or("")).then_some(client)
    }

    fn token(&self, req: &Request) -> Response {
        let err = |status: u16, what: &str| Response::json(status, obj(vec![("error", json!(what))]));
        let form = parse_form(&String::from_utf8_lossy(&req.body));
        let Some(client) = self.client_auth(req, &form) else { return err(401, "invalid_client") };
        let client_id = client["client_id"].as_str().unwrap_or("").to_string();
        let field = |k: &str| form.get(k).map(|s| s.as_str()).unwrap_or("");
        match field("grant_type") {
            "authorization_code" => {
                let rec = lock(&self.codes).remove(field("code"));
                let Some(rec) = rec.filter(|r| r.exp >= state::now() && r.params.get("client_id") == Some(&client_id)) else {
                    return err(400, "invalid_grant");
                };
                let stored = |k: &str| rec.params.get(k).map(|s| s.as_str()).unwrap_or("");
                if !field("redirect_uri").is_empty() && field("redirect_uri") != stored("redirect_uri") {
                    return err(400, "invalid_grant");
                }
                if !stored("code_challenge").is_empty() {
                    let calc = crypto::base64url(&crypto::sha256(field("code_verifier").as_bytes()));
                    if !crypto::constant_time_eq(&calc, stored("code_challenge")) {
                        return Response::json(400, obj(vec![("error", json!("invalid_grant")), ("error_description", json!("PKCE"))]));
                    }
                }
                Response::json(200, self.issue_tokens(&client_id)).with("cache-control", "no-store")
            }
            "refresh_token" => {
                let rt = field("refresh_token");
                let mut store = lock(&self.store);
                let valid = Mcp::token_record(&store, rt, "refresh").filter(|r| r["client_id"].as_str() == Some(&client_id));
                if valid.is_none() {
                    return err(400, "invalid_grant");
                }
                if let Some(Value::Object(tokens)) = store.get_mut("tokens") {
                    tokens.shift_remove(&crypto::sha256_hex(rt)); // rotate
                }
                let tokens = self.issue_tokens_in(&mut store, &client_id);
                Response::json(200, tokens).with("cache-control", "no-store")
            }
            _ => err(400, "unsupported_grant_type"),
        }
    }

    // ------------------------------------------------------------ approval

    /// Whether the tool call may run (asks on this PC when the approval mode says so).
    pub fn approve(&self, tool: &str, args: &Map<String, Value>) -> bool {
        let mode = self.cfg("approval");
        if mode == "none" || (mode == "readonly" && READ_ONLY.contains(&tool)) {
            return true;
        }
        let payload = py::json_dumps(&Value::Object(args.clone()));
        let _one_popup = lock(&self.approval);
        if lock(&self.allowed_until).get(tool).copied().unwrap_or(0.0) > state::now() {
            return true;
        }
        let code = (self.approver)(tool, &payload);
        if code == 2 {
            lock(&self.allowed_until).insert(tool.to_string(), state::now() + 1800.0);
        }
        code == 0 || code == 2
    }

    // ------------------------------------------------------------ mcp protocol

    fn tools_list(&self) -> Value {
        let all_read_only = self.cfg("web_confirm") == "off";
        let tools: Vec<Value> = catalogue()
            .iter()
            .filter(|t| tools::available(&t.name))
            .map(|t| {
                let ro = all_read_only || READ_ONLY.contains(&t.name.as_str());
                obj(vec![
                    ("annotations", obj(vec![("destructiveHint", json!(!ro)), ("readOnlyHint", json!(ro))])),
                    ("description", json!(description(t))),
                    ("inputSchema", t.schema.clone()),
                    ("name", json!(t.name)),
                ])
            })
            .collect();
        obj(vec![("tools", Value::Array(tools))])
    }

    /// `run_tool`: Ok(content blocks) or Err(text of a tool error).
    fn run_tool(&self, name: &str, args: Map<String, Value>) -> Result<Vec<Value>, String> {
        log(&format!("{name} {}", py::prefix(&py::json_dumps(&Value::Object(args.clone())), 200)));
        if !self.approve(name, &args) {
            log(&format!("   ⛔ {}", lang::t("mcpDenied", &[("tool", name)])));
            return Err(DENIED.to_string());
        }
        match tools::call_tool(&json!(name), args) {
            Ok((text, image)) => {
                let mut content = vec![obj(vec![("text", json!(text)), ("type", json!("text"))])];
                if let Some(data) = image {
                    content.push(obj(vec![("data", json!(data)), ("mimeType", json!("image/jpeg")), ("type", json!("image"))]));
                }
                Ok(content)
            }
            Err(Error::Tool(msg)) => Err(msg),
            Err(Error::FailSafe) => Err(py::FAILSAFE_HTTP.to_string()),
            Err(Error::Type(msg)) => Err(format!("Invalid arguments: {msg}")),
            Err(Error::Exc(kind, msg)) => Err(format!("Error: {kind}: {msg}")),
        }
    }

    fn tools_call(&self, params: &Value) -> Result<Value, Value> {
        let Some(name) = params["name"].as_str() else {
            return Err(obj(vec![("code", json!(-32602)), ("message", json!("Invalid request parameters")), ("data", json!(""))]));
        };
        let failed = |text: String| {
            obj(vec![("content", json!([obj(vec![("text", json!(text)), ("type", json!("text"))])])), ("isError", json!(true))])
        };
        let Some(tool) = catalogue().iter().find(|t| t.name == name) else {
            return Ok(failed(format!("Unknown tool: {name}")));
        };
        // a tool this system does not have is turned away before anyone is asked to approve it
        if !tools::available(name) {
            return Ok(failed(format!("Error executing tool {name}: {}", tools::unavailable(name).http_message())));
        }
        let empty = Map::new();
        let given = params["arguments"].as_object().unwrap_or(&empty);
        let args = match validate(tool, given) {
            Ok(args) => args,
            Err(text) => {
                eprintln!("Tool '{name}' rejected arguments");
                return Ok(failed(format!("Error executing tool {name}: {text}")));
            }
        };
        Ok(match self.run_tool(name, args) {
            Ok(content) => obj(vec![("content", Value::Array(content)), ("isError", json!(false))]),
            Err(text) => {
                eprintln!("Tool '{name}' failed: {text}");
                failed(format!("Error executing tool {name}: {text}"))
            }
        })
    }

    fn dispatch(&self, method: &str, params: &Value) -> Result<Value, Value> {
        match method {
            "initialize" => {
                if !(params["protocolVersion"].is_string() && params["capabilities"].is_object() && params["clientInfo"].is_object()) {
                    return Err(obj(vec![("code", json!(-32602)), ("message", json!("Invalid request parameters")), ("data", json!(""))]));
                }
                let wanted = params["protocolVersion"].as_str().unwrap_or("");
                let version = if PROTOCOL_VERSIONS.contains(&wanted) { wanted } else { LATEST_PROTOCOL };
                Ok(obj(vec![
                    (
                        "capabilities",
                        obj(vec![
                            ("prompts", obj(vec![("listChanged", json!(false))])),
                            ("resources", obj(vec![("listChanged", json!(false)), ("subscribe", json!(false))])),
                            ("tools", obj(vec![("listChanged", json!(false))])),
                        ]),
                    ),
                    ("instructions", json!(INSTRUCTIONS)),
                    ("protocolVersion", json!(version)),
                    (
                        "serverInfo",
                        obj(vec![("name", json!("GeminiForge")), ("title", json!("GeminiForge")), ("version", json!(state::VERSION))]),
                    ),
                ]))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.tools_list()),
            "tools/call" => self.tools_call(params),
            "resources/list" => Ok(obj(vec![("resources", json!([]))])),
            "prompts/list" => Ok(obj(vec![("prompts", json!([]))])),
            "resources/templates/list" => Ok(obj(vec![("resourceTemplates", json!([]))])),
            other => Err(obj(vec![("code", json!(-32601)), ("message", json!("Method not found")), ("data", json!(other))])),
        }
    }

    fn rpc_error(status: u16, id: Value, code: i64, message: String) -> Response {
        Response::json(
            status,
            obj(vec![("jsonrpc", json!("2.0")), ("id", id), ("error", obj(vec![("code", json!(code)), ("message", json!(message))]))]),
        )
    }

    fn mcp_post(&self, req: &Request) -> Response {
        let accept = req.header("accept").to_lowercase();
        let accepts = |kind: &str| accept.contains(kind) || accept.contains("*/*");
        if !(accepts("application/json") && accepts("text/event-stream")) {
            return Mcp::rpc_error(
                406,
                Value::Null,
                -32600,
                "Not Acceptable: Client must accept both application/json and text/event-stream".to_string(),
            );
        }
        if !req.header("content-type").to_lowercase().starts_with("application/json") {
            return Response::new(400, "", b"Invalid Content-Type header".to_vec());
        }
        let msg: Value = match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(e) => return Mcp::rpc_error(400, Value::Null, -32700, format!("Parse error: {e}")),
        };
        let shape_ok = msg.is_object()
            && msg["jsonrpc"].as_str() == Some("2.0")
            && (msg["method"].is_string() || msg.get("result").is_some() || msg.get("error").is_some());
        if !shape_ok {
            return Mcp::rpc_error(
                400,
                Value::Null,
                -32602,
                "Validation error: the body must be one JSON-RPC 2.0 request, notification or response".to_string(),
            );
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let version = req.header("mcp-protocol-version");
        if !version.is_empty() && !PROTOCOL_VERSIONS.contains(&version) {
            return Mcp::rpc_error(
                400,
                id,
                -32602,
                format!("Bad Request: Unsupported protocol version: {version}. Supported versions: {}", PROTOCOL_VERSIONS.join(", ")),
            );
        }
        let Some(method) = msg["method"].as_str().filter(|_| !id.is_null()) else {
            // a notification or a response: nothing to answer
            return Response::new(202, "application/json", Vec::new());
        };
        let answer = match self.dispatch(method, &msg["params"]) {
            Ok(result) => obj(vec![("jsonrpc", json!("2.0")), ("id", id), ("result", result)]),
            Err(error) => obj(vec![("jsonrpc", json!("2.0")), ("id", id), ("error", error)]),
        };
        let body = format!("event: message\r\ndata: {}\r\n\r\n", py::json_compact(&answer));
        Response::new(200, "text/event-stream", body.into_bytes()).with("cache-control", "no-cache, no-transform")
    }

    fn unauthorized(&self, req: &Request) -> Response {
        let meta = format!("{}/.well-known/oauth-protected-resource", self.public_url(req));
        Response::new(401, "application/json", b"{\"error\": \"invalid_token\"}".to_vec())
            .with("www-authenticate", &format!("Bearer resource_metadata=\"{meta}\", scope=\"geminiforge\""))
    }

    fn is_mcp(path: &str) -> bool {
        path == "/mcp" || path.starts_with("/mcp/")
    }

    fn route(&self, req: &Request) -> Response {
        if Mcp::is_mcp(&req.path) {
            if !self.authorized(req) {
                return self.unauthorized(req);
            }
            if req.path != "/mcp" {
                let host = if req.header("host").is_empty() { format!("127.0.0.1:{}", self.port) } else { req.header("host").to_string() };
                return Response { status: 307, headers: vec![("location".to_string(), format!("http://{host}/mcp"))], body: Vec::new(), stream: false };
            }
            return match req.method.as_str() {
                "POST" => {
                    if req.body.len() > MAX_MCP_BODY.load(Ordering::Relaxed) {
                        return Response::new(413, "application/json", b"{\"error\": \"payload_too_large\"}".to_vec());
                    }
                    self.rpc_log("→", &req.body);
                    self.mcp_post(req)
                }
                "GET" => {
                    let mut r = Response::new(200, "text/event-stream", Vec::new()).with("cache-control", "no-cache, no-transform");
                    r.stream = true;
                    r
                }
                "DELETE" => Mcp::rpc_error(405, Value::Null, -32600, "Method Not Allowed: Session termination not supported".to_string()),
                _ => Mcp::rpc_error(405, Value::Null, -32600, "Method Not Allowed".to_string()),
            };
        }
        let allowed: &[&str] = match req.path.as_str() {
            "/"
            | "/.well-known/oauth-protected-resource"
            | "/.well-known/oauth-protected-resource/mcp"
            | "/.well-known/oauth-authorization-server"
            | "/.well-known/oauth-authorization-server/mcp"
            | "/.well-known/openid-configuration" => &["GET", "HEAD"],
            "/register" | "/token" => &["POST"],
            "/authorize" => &["GET", "HEAD", "POST"],
            _ => return Response::text(404, "Not Found"),
        };
        if !allowed.contains(&req.method.as_str()) {
            return Response::text(405, "Method Not Allowed").with("allow", &allowed.join(", "));
        }
        match req.path.as_str() {
            "/" => Response::text(200, "GeminiForge MCP server. Endpoint: /mcp"),
            "/register" => self.register(req),
            "/authorize" => self.authorize(req),
            "/token" => self.token(req),
            p if p.contains("oauth-protected-resource") => self.resource_metadata(req),
            _ => self.server_metadata(req),
        }
    }

    /// Answer one request (everything except writing it to the socket).
    pub fn handle(&self, req: &Request) -> Response {
        let mut res = self.route(req);
        if req.method == "HEAD" {
            res.body.clear();
            res.stream = false;
        }
        self.access_log(req, res.status);
        if Mcp::is_mcp(&req.path) && !res.body.is_empty() {
            let shown = &res.body[..res.body.len().min(4000)];
            self.rpc_log("←", shown);
        }
        res
    }

    // ------------------------------------------------------------ logs

    fn append_access_log(&self, line: &str) {
        let _guard = lock(&self.log_file);
        let path = self.dir.join("mcp_access.log");
        let too_big = std::fs::metadata(&path).map(|m| m.len() as usize >= MAX_LOG_BYTES.load(Ordering::Relaxed)).unwrap_or(false);
        if too_big {
            let _ = std::fs::rename(&path, self.dir.join("mcp_access.log.1"));
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "{line}");
        }
    }

    fn access_log(&self, req: &Request, status: u16) {
        let ua = py::prefix(req.header("user-agent"), 60).to_string();
        let query = if req.query.is_empty() { String::new() } else { format!("?{}", py::prefix(&req.query, 120)) };
        let line = format!("{} {} {}{} -> {} [{}] {}", stamp(), req.method, req.path, query, status, req.header("x-forwarded-for"), ua);
        self.append_access_log(&line);
        if req.path != "/mcp" || status != 200 {
            println!("  {line}");
            let _ = std::io::stdout().flush();
        }
    }

    /// Note which JSON-RPC methods Gemini calls and what came back.
    pub fn rpc_log(&self, direction: &str, raw: &[u8]) {
        let text = String::from_utf8_lossy(raw);
        let text = py::strip(&text);
        if text.is_empty() {
            return;
        }
        let lines = py::splitlines(text);
        let non_empty: Vec<&&str> = lines.iter().filter(|l| !py::strip(l).is_empty()).collect();
        if !non_empty.is_empty() && non_empty.iter().all(|l| l.trim_start().starts_with(':')) {
            return; // keep-alive pings
        }
        let data: Vec<&str> = lines.iter().filter_map(|l| l.strip_prefix("data:")).collect();
        let chunks: Vec<&str> = if data.is_empty() { vec![text] } else { data };
        let summary = (|| -> Option<String> {
            let mut msgs: Vec<String> = Vec::new();
            for chunk in chunks {
                let parsed: Value = serde_json::from_str(chunk).ok()?;
                let items: Vec<Value> = match parsed {
                    Value::Array(a) => a,
                    other => vec![other],
                };
                for o in items {
                    let o = o.as_object()?;
                    if let Some(method) = o.get("method") {
                        let name = o.get("params").and_then(|p| p.get("name")).filter(|n| py::truthy(n)).map(py::py_str);
                        msgs.push(match name {
                            Some(n) => format!("{} {n}", py::py_str(method)),
                            None => py::py_str(method),
                        });
                    } else if let Some(error) = o.get("error") {
                        msgs.push(format!("ERROR {}", py::prefix(&py::json_dumps(error), 300)));
                    } else if let Some(r) = o.get("result") {
                        match r.as_object() {
                            Some(r) if r.get("isError").map(py::truthy).unwrap_or(false) => {
                                let content = r.get("content").cloned().unwrap_or(Value::Null);
                                msgs.push(format!("tool error {}", py::prefix(&py::json_dumps(&content), 200)));
                            }
                            Some(r) => {
                                let keys: Vec<&str> = r.keys().take(4).map(|k| k.as_str()).collect();
                                msgs.push(format!("result {}", keys.join(",")));
                            }
                            None => msgs.push("result".to_string()),
                        }
                    }
                }
            }
            Some(msgs.join(" | "))
        })();
        let line = match summary {
            Some(s) => format!("{}   {direction} {s}", stamp()),
            None => format!("{}   {direction} (raw) {}", stamp(), py::prefix(text, 200)),
        };
        self.append_access_log(&line);
    }
}

// ---------------------------------------------------------------- sockets

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        302 => "Found",
        307 => "Temporary Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        _ => "",
    }
}

fn write_head(stream: &mut TcpStream, res: &Response, length: Option<usize>) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\nserver: GeminiForge-MCP/{}\r\n", res.status, reason(res.status), state::VERSION);
    for (k, v) in &res.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(n) = length {
        head.push_str(&format!("content-length: {n}\r\n"));
    }
    head.push_str("connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())
}

fn read_chunked(reader: &mut BufReader<TcpStream>, limit: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        reader.by_ref().take(64).read_line(&mut line).ok()?;
        let size = usize::from_str_radix(line.trim().split(';').next()?.trim(), 16).ok()?;
        if size == 0 {
            return Some(body);
        }
        if body.len() + size > limit + 1 {
            body.resize(limit + 1, 0);
            return Some(body);
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..]).ok()?;
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).ok()?;
    }
}

fn serve_one(mcp: &Mcp, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(30)));
    let Ok(read_half) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read_half);
    let mut first = String::new();
    if reader.by_ref().take(16384).read_line(&mut first).is_err() {
        return;
    }
    let words: Vec<&str> = first.split_whitespace().collect();
    if words.len() != 3 || !words[2].starts_with("HTTP/") {
        return;
    }
    let mut req = Request::new(words[0], words[1]);
    let mut head_bytes = first.len();
    loop {
        let mut line = String::new();
        if reader.by_ref().take(16384).read_line(&mut line).is_err() {
            return;
        }
        head_bytes += line.len();
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if head_bytes > 65536 || req.headers.len() >= 100 {
            let res = Response::text(431, "Request Header Fields Too Large");
            let _ = write_head(&mut stream, &res, Some(res.body.len())).and_then(|_| stream.write_all(&res.body));
            return;
        }
        if let Some((k, v)) = line.split_once(':') {
            req.headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    // The token is checked before a single byte of the body is read.
    let is_mcp = Mcp::is_mcp(&req.path);
    let skip_body = is_mcp && !mcp.authorized(&req);
    if !skip_body {
        let limit = if is_mcp { MAX_MCP_BODY.load(Ordering::Relaxed) } else { MAX_AUTH_BODY };
        if req.header("transfer-encoding").to_lowercase().contains("chunked") {
            match read_chunked(&mut reader, limit) {
                Some(body) => req.body = body,
                None => return,
            }
        } else {
            let n: usize = req.header("content-length").trim().parse().unwrap_or(0);
            // one byte over the limit is enough to answer "too large" without reading it all
            let take = n.min(limit + 1);
            let mut body = Vec::new();
            if reader.by_ref().take(take as u64).read_to_end(&mut body).is_err() {
                return;
            }
            req.body = body;
        }
        if !is_mcp && req.body.len() > limit {
            let res = Response::text(413, "Content Too Large");
            let _ = write_head(&mut stream, &res, Some(res.body.len())).and_then(|_| stream.write_all(&res.body));
            return;
        }
    }
    let res = mcp.handle(&req);
    if res.stream {
        // the stand-alone event stream: nothing is ever sent on it but keep-alive comments
        if mcp.streams.fetch_add(1, Ordering::SeqCst) < MAX_STREAMS && write_head(&mut stream, &res, None).is_ok() {
            let _ = stream.flush();
            loop {
                state::sleep(15.0);
                let t = ffi::system_time();
                let ping = format!(
                    ": ping - {:04}-{:02}-{:02} {:02}:{:02}:{:02}+00:00\r\n\r\n",
                    t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
                );
                if stream.write_all(ping.as_bytes()).and_then(|_| stream.flush()).is_err() {
                    break;
                }
            }
        }
        mcp.streams.fetch_sub(1, Ordering::SeqCst);
        return;
    }
    if write_head(&mut stream, &res, Some(res.body.len())).and_then(|_| stream.write_all(&res.body)).is_err() {
        return;
    }
    let _ = stream.flush();
    // let the client finish sending before the close, so the close is not a reset
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let mut sink = [0u8; 8192];
    let mut left = 1024 * 1024usize;
    while left > 0 {
        match reader.read(&mut sink) {
            Ok(n) if n > 0 => left = left.saturating_sub(n),
            _ => break,
        }
    }
}

/// Serve forever on an already bound listener (one thread per connection, bounded).
pub fn serve(mcp: Arc<Mcp>, listener: TcpListener) {
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            continue; // dropped: too many connections at once
        }
        active.fetch_add(1, Ordering::SeqCst);
        let (mcp, active) = (mcp.clone(), active.clone());
        let spawned = std::thread::Builder::new().name("gf-mcp".to_string()).spawn(move || {
            serve_one(&mcp, stream);
            active.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            // the closure (and its counter decrement) never ran
        }
    }
}

/// `geminiforge-server mcp`: the whole program.
pub fn run() {
    let port: u16 = std::env::var("GF_MCP_PORT").ok().and_then(|p| p.trim().parse().ok()).unwrap_or(DEFAULT_PORT);
    let listener = match TcpListener::bind((state::HOST, port)) {
        Ok(l) => l,
        Err(_) => {
            println!("{}", lang::t("mcpPortInUse", &[("port", &port.to_string())]));
            use std::io::IsTerminal;
            if std::io::stdin().is_terminal() {
                print!("{}", lang::t("pressEnterToExit", &[]));
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
            }
            return;
        }
    };
    let mcp = Arc::new(Mcp::new(&default_dir(), port));
    // GF_TUNNEL overrides "tunnel" in mcp_config.json
    let tunnel = std::env::var("GF_TUNNEL").ok().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| mcp.cfg("tunnel"));
    let quick = tunnel.trim().eq_ignore_ascii_case("cloudflared");
    if quick {
        println!("{}", lang::t("mcpTunnelStarting", &[]));
        let _ = std::io::stdout().flush();
        match start_quick_tunnel(port) {
            // for this run only: the saved public_url stays as it is
            Ok(url) => mcp.set_cfg("public_url", &url),
            Err(msg) => println!("{msg}"),
        }
    }
    let url = match mcp.cfg("public_url") {
        u if u.is_empty() => lang::t("mcpNoTailscale", &[]),
        u => u,
    };
    let bar = "=".repeat(60);
    println!("{bar}");
    println!(" {}", lang::t("mcpTitle", &[]));
    println!("{bar}");
    println!(" {}", lang::t("mcpUrl", &[("url", &format!("{url}/mcp"))]));
    if quick {
        println!(" {}", lang::t("mcpTunnelQuick", &[]));
    }
    println!(" {}", lang::t("mcpPairCode", &[("code", &mcp.cfg("pairing_code"))]));
    println!(" {}", lang::t("mcpApproval", &[("mode", &mcp.cfg("approval"))]));
    println!(" {}", lang::t("mcpWebConfirm", &[("mode", &mcp.cfg("web_confirm"))]));
    #[cfg(not(windows))]
    if mcp.cfg("approval") != "none" {
        if let Some(why) = crate::unix::popup::missing() {
            println!(" {}", lang::t("mcpNoPopup", &[("why", &why)]));
        }
    }
    println!(" {}", lang::t(if cfg!(windows) { "mcpStop" } else { "mcpStopPlain" }, &[]));
    if state::fast_wait() {
        println!(" Fast waits: on (GF_FAST_WAIT=1)");
    }
    println!("{bar}");
    let _ = std::io::stdout().flush();
    serve(mcp, listener);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_tunnel_address_is_read_from_the_log() {
        let line = "2026-11-28T01:02:03Z INF |  https://calm-river-mountain-sky.trycloudflare.com                      |";
        assert_eq!(quick_tunnel_url(line).as_deref(), Some("https://calm-river-mountain-sky.trycloudflare.com"));
        let api = "ERR failed to request quick Tunnel: Post \"https://api.trycloudflare.com/tunnel\": dial tcp";
        assert_eq!(quick_tunnel_url(api), None);
        assert_eq!(quick_tunnel_url("INF Requesting new quick Tunnel on trycloudflare.com..."), None);
        assert_eq!(quick_tunnel_url("see https://example.com and https://a-b.trycloudflare.com"), Some("https://a-b.trycloudflare.com".into()));
    }

    #[test]
    fn url_and_form_helpers_follow_python() {
        assert_eq!(url_parts("https://gemini.google.com/cb?x=1"), ("https".to_string(), "gemini.google.com".to_string()));
        assert_eq!(url_parts("HTTPS://Host:8443"), ("https".to_string(), "Host:8443".to_string()));
        assert_eq!(url_parts("https:///cb"), ("https".to_string(), String::new()));
        assert_eq!(url_parts("no scheme"), (String::new(), String::new()));
        assert_eq!(quote_plus("https://gf-test.example:10000"), "https%3A%2F%2Fgf-test.example%3A10000");
        assert_eq!(quote_plus("st&1 あ"), "st%261+%E3%81%82");
        let form = parse_form("a=1&b=x+y%26z&a=2&c&d=%E3%81%82%zz");
        assert_eq!(form["a"], "2");
        assert_eq!(form["b"], "x y&z");
        assert_eq!(form["c"], "");
        assert_eq!(form["d"], "あ%zz");
        assert_eq!(html_escape("<a href=\"x\">&'</a>"), "&lt;a href=&quot;x&quot;&gt;&amp;&#x27;&lt;/a&gt;");
    }

    #[test]
    fn arguments_are_validated_like_pydantic() {
        let tool = |name: &str| catalogue().iter().find(|t| t.name == name).unwrap();
        let args = |v: Value| v.as_object().cloned().unwrap();
        // defaults are filled in, omitted optional arguments stay out
        let out = validate(tool("click"), &args(json!({"x": "5", "y": 7.0}))).unwrap();
        assert_eq!(
            Value::Object(out),
            json!({"x": 5, "y": 7, "button": "left", "clicks": 1, "background": false, "cursor_jump": false, "restore_focus": false, "screenshot": true})
        );
        let out = validate(tool("job_status"), &args(json!({"id": null}))).unwrap();
        // a default is passed on as written (pydantic does not convert defaults)
        assert_eq!(Value::Object(out), json!({"seconds": 0}));
        let out = validate(tool("wait"), &args(json!({"seconds": "0.05", "bogus": 1}))).unwrap();
        assert_eq!(Value::Object(out), json!({"seconds": 0.05}));
        assert_eq!(
            validate(tool("read_file"), &args(json!({}))).unwrap_err(),
            "1 validation error for read_fileArguments\npath\n  Field required [type=missing, input_value={}, input_type=dict]\n    For further information visit https://errors.pydantic.dev/2.14/v/missing"
        );
        assert_eq!(
            validate(tool("read_file"), &args(json!({"path": "x", "limit": "abc"}))).unwrap_err(),
            "1 validation error for read_fileArguments\nlimit\n  Input should be a valid integer, unable to parse string as an integer [type=int_parsing, input_value='abc', input_type=str]\n    For further information visit https://errors.pydantic.dev/2.14/v/int_parsing"
        );
        assert_eq!(
            validate(tool("wait_for"), &args(json!({"window": "w", "timeout": 0.3}))).unwrap_err(),
            "1 validation error for wait_forArguments\ntimeout\n  Input should be a valid integer, got a number with a fractional part [type=int_from_float, input_value=0.3, input_type=float]\n    For further information visit https://errors.pydantic.dev/2.14/v/int_from_float"
        );
        let two = validate(tool("click_elements"), &args(json!({"names": ["a", 5]}))).unwrap_err();
        assert!(two.starts_with("2 validation errors for click_elementsArguments\nwindow_id\n  Field required"), "{two}");
        assert!(two.contains("\nnames.1\n  Input should be a valid string [type=string_type, input_value=5, input_type=int]"), "{two}");
    }

    #[test]
    fn catalogue_covers_every_tool_in_order() {
        let names: Vec<&str> = catalogue().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, tools::TOOLS.iter().map(|t| t.name).collect::<Vec<_>>());
        for (spec, def) in catalogue().iter().zip(tools::TOOLS) {
            let mut expected: Vec<&str> = def.params.iter().map(|(n, _)| *n).collect();
            if tools::SCREEN_ACTIONS.contains(&def.name) {
                expected.push("screenshot");
            }
            let got: Vec<&str> = spec.params.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(got, expected, "{}", def.name);
            for (p, (_, required)) in spec.params.iter().zip(def.params) {
                assert_eq!(p.required, *required, "{}.{}", def.name, p.name);
            }
            assert!(!spec.description.is_empty());
        }
        for name in READ_ONLY {
            assert!(tools::find(name).is_some(), "{name}");
        }
        if cfg!(windows) {
            assert!(INSTRUCTIONS.starts_with("GeminiForge lets you operate the user's Windows PC"));
        } else {
            assert!(INSTRUCTIONS.starts_with("GeminiForge lets you operate the user's macOS or Linux computer"));
            assert!(description(catalogue().iter().find(|t| t.name == "run").unwrap()).contains("bash"));
        }
    }
}
