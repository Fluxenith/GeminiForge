//! The HTTP side: 127.0.0.1 only, HTTP/1.0, one thread per connection,
//! same paths, status codes and JSON bodies as `Handler` in server.py.

use crate::ffi;
use crate::lang;
use crate::py::{self, Error};
use crate::state::{self, MAX_HTTP_BODY, VERSION};
use crate::tools;
use serde_json::{json, Map, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const BAD_REQUEST: &str = "Bad request syntax or unsupported method";
const TOO_LARGE: &str = "The server is unwilling to process the request because its header fields are too large";

fn http_date() -> String {
    let t = ffi::system_time();
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[(t.wDayOfWeek as usize) % 7],
        t.wDay,
        MONTHS[(t.wMonth as usize + 11) % 12],
        t.wYear,
        t.wHour,
        t.wMinute,
        t.wSecond
    )
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Content Too Large",
        414 => "URI Too Long",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        _ => "",
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Request {
    /// `self.headers.get(name, "")`
    fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

struct Conn {
    stream: TcpStream,
    port: u16,
}

impl Conn {
    fn write_response(&mut self, code: u16, extra: &[(&str, String)], body: &[u8]) {
        let mut head = format!(
            "HTTP/1.0 {code} {}\r\nServer: GeminiForge/{VERSION} Rust\r\nDate: {}\r\n",
            reason(code),
            http_date()
        );
        for (k, v) in extra {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let mut out = head.into_bytes();
        out.extend_from_slice(body);
        let _ = self.stream.write_all(&out);
        let _ = self.stream.flush();
    }

    /// `_send(code, obj)`
    fn send(&mut self, req: &Request, code: u16, obj: Value) {
        let body = py::json_dumps(&obj).into_bytes();
        let mut headers = vec![
            ("Content-Type", "application/json; charset=utf-8".to_string()),
            ("Content-Length", body.len().to_string()),
        ];
        let origin = req.header("Origin");
        if origin.starts_with("chrome-extension://") {
            headers.push(("Access-Control-Allow-Origin", origin.to_string()));
        }
        self.write_response(code, &headers, &body);
    }

    /// `send_error(code, message)` of http.server (an HTML page).
    fn send_error(&mut self, code: u16, message: &str, explain: &str) {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let body = format!(
            "<!DOCTYPE HTML>\n<html lang=\"en\">\n    <head>\n        <meta charset=\"utf-8\">\n        <title>Error response</title>\n    </head>\n    <body>\n        <h1>Error response</h1>\n        <p>Error code: {code}</p>\n        <p>Message: {}.</p>\n        <p>Error code explanation: {code} - {}.</p>\n    </body>\n</html>\n",
            esc(message),
            esc(explain)
        )
        .into_bytes();
        let headers = [
            ("Connection", "close".to_string()),
            ("Content-Type", "text/html;charset=utf-8".to_string()),
            ("Content-Length", body.len().to_string()),
        ];
        self.write_response(code, &headers, &body);
    }

    fn allowed_host(&self, req: &Request) -> bool {
        let host = req.header("Host");
        host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port)
    }

    /// Blocks DNS-rebinding and calls from ordinary web pages: only the
    /// extension (chrome-extension:// origin) may run tools.
    fn allowed(&self, req: &Request) -> bool {
        self.allowed_host(req) && req.header("Origin").starts_with("chrome-extension://")
    }
}

fn read_line(reader: &mut BufReader<TcpStream>, limit: usize) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    let mut taken = reader.by_ref().take(limit as u64 + 1);
    taken.read_until(b'\n', &mut line).ok()?;
    Some(line)
}

fn text_of(bytes: &[u8]) -> String {
    // http.server decodes the request head as iso-8859-1
    bytes.iter().map(|b| *b as char).collect()
}

fn version_ok(version: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    version.strip_prefix("HTTP/").and_then(|v| v.split_once('.')).map(|(a, b)| digits(a) && digits(b)).unwrap_or(false)
}

fn handle(stream: TcpStream, port: u16) {
    let Ok(read_half) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read_half);
    let mut conn = Conn { stream, port };
    respond(&mut conn, &mut reader);
    finish(conn, reader);
}

/// Close without resetting the connection: say we are done writing, then let
/// the client finish sending (a body we did not read would otherwise turn the
/// close into a reset that can wipe out the response before it is read).
fn finish(conn: Conn, mut reader: BufReader<TcpStream>) {
    let _ = conn.stream.shutdown(std::net::Shutdown::Write);
    let _ = conn.stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let mut sink = [0u8; 16384];
    let mut left: usize = 4 * 1024 * 1024;
    while left > 0 {
        match reader.read(&mut sink) {
            Ok(n) if n > 0 => left = left.saturating_sub(n),
            _ => break,
        }
    }
}

fn respond(conn: &mut Conn, reader: &mut BufReader<TcpStream>) {

    let Some(raw) = read_line(reader, 65536) else { return };
    if raw.len() > 65536 {
        conn.send_error(414, "Request-URI Too Long", "URI is too long");
        return;
    }
    let line = text_of(&raw);
    let line = line.trim_end_matches(['\r', '\n']);
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.is_empty() {
        return;
    }
    if words.len() >= 3 && !version_ok(words[words.len() - 1]) {
        let msg = format!("Bad request version ({})", py::repr_str(words[words.len() - 1]));
        conn.send_error(400, &msg, BAD_REQUEST);
        return;
    }
    if !(2..=3).contains(&words.len()) {
        conn.send_error(400, &format!("Bad request syntax ({})", py::repr_str(line)), BAD_REQUEST);
        return;
    }
    if words.len() == 2 && words[0] != "GET" {
        conn.send_error(400, &format!("Bad HTTP/0.9 request type ({})", py::repr_str(words[0])), BAD_REQUEST);
        return;
    }
    let mut req = Request { method: words[0].to_string(), path: words[1].to_string(), headers: Vec::new() };

    loop {
        let Some(raw) = read_line(reader, 65536) else { return };
        if raw.len() > 65536 {
            conn.send_error(431, "Line too long", TOO_LARGE);
            return;
        }
        let line = text_of(&raw);
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if req.headers.len() >= 100 {
            conn.send_error(431, "Too many headers", TOO_LARGE);
            return;
        }
        if let Some((k, v)) = line.split_once(':') {
            req.headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    match req.method.as_str() {
        "OPTIONS" => conn.send(&req, 403, json!({"ok": false})),
        "GET" => do_get(conn, &req),
        "POST" => do_post(conn, &req, reader),
        other => {
            let msg = format!("Unsupported method ({})", py::repr_str(other));
            conn.send_error(501, &msg, "Server does not support this operation");
        }
    }
}

fn do_get(conn: &mut Conn, req: &Request) {
    if req.path == "/health" && conn.allowed_host(req) {
        let mut obj = Map::new();
        obj.insert("ok".into(), json!(true));
        obj.insert("version".into(), json!(VERSION));
        obj.insert("cwd".into(), json!(state::cwd()));
        obj.insert("tools".into(), json!(tools::tool_names()));
        obj.insert("languageSync".into(), json!(true));
        if !cfg!(windows) {
            // "macos" or "linux"; the extension takes a missing os for Windows
            obj.insert("os".into(), json!(std::env::consts::OS));
        }
        if state::fast_wait() {
            // only present with GF_FAST_WAIT=1, so the default answer stays identical to server.py
            obj.insert("fastWait".into(), json!(true));
        }
        return conn.send(req, 200, Value::Object(obj));
    }
    conn.send(req, 404, json!({"ok": false}));
}

fn read_body(reader: &mut BufReader<TcpStream>, n: usize) -> Vec<u8> {
    let mut body = Vec::new();
    let _ = reader.take(n as u64).read_to_end(&mut body);
    body
}

fn err_obj(message: &str) -> Value {
    let mut obj = Map::new();
    obj.insert("ok".into(), json!(false));
    obj.insert("error".into(), json!(message));
    Value::Object(obj)
}

fn log_time() -> String {
    let t = ffi::local_time();
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

fn do_post(conn: &mut Conn, req: &Request, reader: &mut BufReader<TcpStream>) {
    if (req.path != "/tool" && req.path != "/language") || !conn.allowed(req) {
        return conn.send(req, 403, err_obj("forbidden"));
    }
    let length = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("0");
    let Ok(n) = py::int(&Value::String(length.to_string())) else {
        return conn.send(req, 400, err_obj("invalid Content-Length"));
    };
    if n <= 0 {
        return conn.send(req, 400, err_obj("missing body"));
    }
    let n = n as usize;
    if req.path == "/language" {
        if n > 128 {
            return conn.send(req, 400, json!({"ok": false}));
        }
        let body = read_body(reader, n);
        let language = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v.get("language").and_then(|l| l.as_str().map(|s| s.to_string())))
            .filter(|l| lang::LANGUAGES.contains(&l.as_str()));
        let Some(language) = language else {
            return conn.send(req, 400, json!({"ok": false}));
        };
        return match lang::save_language(&language) {
            Ok(()) => conn.send(req, 200, json!({"ok": true})),
            Err(e) => conn.send(req, 200, err_obj(&py::os_error(&e, None).http_message())),
        };
    }
    if n > MAX_HTTP_BODY.load(Ordering::Relaxed) {
        return conn.send(req, 413, err_obj("request too large"));
    }
    let body = read_body(reader, n);
    let parsed = std::str::from_utf8(&body).ok().and_then(|s| serde_json::from_str::<Value>(s).ok());
    let Some(parsed) = parsed else {
        return conn.send(req, 400, err_obj("invalid JSON"));
    };
    let Value::Object(mut obj) = parsed else {
        return conn.send(req, 400, err_obj("JSON body must be an object"));
    };
    let name = obj.remove("tool").unwrap_or(Value::Null);
    let args = match obj.remove("args") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(a)) => Ok(a),
        Some(_) => Err(Error::tool("args must be a JSON object")),
    };
    let result = args.and_then(|args| {
        let shown = py::json_dumps(&Value::Object(args.clone()));
        println!("[{}] {} {}", log_time(), py::py_str(&name), py::prefix(&shown, 200));
        let _ = std::io::stdout().flush();
        tools::call_tool(&name, args)
    });
    match result {
        Ok((text, image)) => {
            let mut out = Map::new();
            out.insert("ok".into(), json!(true));
            out.insert("output".into(), json!(text));
            out.insert("image".into(), image.map(Value::String).unwrap_or(Value::Null));
            conn.send(req, 200, Value::Object(out));
        }
        Err(e) => {
            if let Error::Exc(kind, msg) = &e {
                eprintln!("{kind}: {msg}");
            }
            conn.send(req, 200, err_obj(&e.http_message()));
        }
    }
}

/// Serve forever on an already bound listener (one thread per connection).
pub fn serve(listener: TcpListener) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let _ = std::thread::Builder::new().name("gf-http".to_string()).spawn(move || handle(stream, port));
    }
}

/// Bind 127.0.0.1:port; fails when the port is taken.
pub fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind((state::HOST, port))
}
