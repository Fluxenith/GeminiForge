//! The MCP entry: OAuth, the protocol, approval and logs (the Rust side of the
//! MCPServerTests in server/test_server_tools.py and server/test_ui_tools.py).
//! Everything runs against temporary folders; no popup is shown.

mod common;

use common::{serial, TempDir};
use geminiforge::mcp::{self, Mcp, Request, Response};
use geminiforge::{crypto, state};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const CONFIG: &str = r#"{
  "public_url": "https://box.ts.net:10000",
  "pairing_code": "12345678",
  "client_id": "geminiforge",
  "client_secret": "test-secret",
  "approval": "readonly",
  "web_confirm": "on"
}"#;

fn server(tmp: &TempDir, popup_code: i32) -> (Mcp, Arc<AtomicUsize>) {
    std::fs::write(tmp.join("mcp_config.json"), CONFIG).unwrap();
    let shown = Arc::new(AtomicUsize::new(0));
    let counter = shown.clone();
    let approver = Box::new(move |_tool: &str, _args: &str| {
        counter.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(50));
        popup_code
    });
    (Mcp::with_approver(tmp.path(), 18767, approver, false), shown)
}

fn post_json(path: &str, body: Value) -> Request {
    Request::new("POST", path).with_header("content-type", "application/json").with_body(body.to_string().as_bytes())
}

fn form(path: &str, fields: &[(&str, &str)]) -> Request {
    let body: Vec<String> = fields.iter().map(|(k, v)| format!("{k}={}", mcp::quote_plus(v))).collect();
    Request::new("POST", path).with_header("content-type", "application/x-www-form-urlencoded").with_body(body.join("&").as_bytes())
}

fn body_json(res: &Response) -> Value {
    serde_json::from_slice(&res.body).unwrap_or(Value::Null)
}

/// The JSON-RPC message inside an event-stream answer.
fn rpc(res: &Response) -> Value {
    let text = res.text_body();
    let data = text.lines().find_map(|l| l.strip_prefix("data: ")).unwrap_or("null");
    serde_json::from_str(data).unwrap_or(Value::Null)
}

fn call(m: &Mcp, token: &str, method: &str, params: Value) -> Response {
    m.handle(
        &post_json("/mcp", json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .with_header("authorization", &format!("Bearer {token}"))
            .with_header("accept", "application/json, text/event-stream"),
    )
}

fn access_token(m: &Mcp) -> String {
    m.issue_tokens("geminiforge")["access_token"].as_str().unwrap().to_string()
}

#[test]
fn register_validation_and_client_eviction() {
    let _g = serial();
    let tmp = TempDir::new("mcp_reg");
    let (m, _) = server(&tmp, 1);
    // Non-dict JSON rejected
    assert_eq!(m.handle(&post_json("/register", json!([]))).status, 400);
    assert_eq!(m.handle(&Request::new("POST", "/register").with_body(b"{nope")).text_body(), "{\"error\":\"invalid_client_metadata\"}");
    // Non-https or missing netloc redirect_uris rejected
    for bad in [json!({"redirect_uris": ["http://example.com/cb"]}), json!({"redirect_uris": ["https:///cb"]}), json!({"client_name": "x"})] {
        let res = m.handle(&post_json("/register", bad));
        assert_eq!((res.status, res.text_body().as_str()), (400, "{\"error\":\"invalid_redirect_uri\"}"));
    }
    let old = mcp::MAX_CLIENTS.swap(2, Ordering::SeqCst);
    let mut cids = Vec::new();
    for i in 0..3 {
        let res = m.handle(&post_json("/register", json!({"redirect_uris": [format!("https://app{i}.example/cb")], "client_name": format!("App{i}")})));
        assert_eq!(res.status, 201);
        let body = body_json(&res);
        assert_eq!(body["token_endpoint_auth_method"], "client_secret_post");
        assert_eq!(body["client_secret"].as_str().unwrap().len(), 43);
        assert_eq!(body["client_secret_expires_at"], 0);
        cids.push(body["client_id"].as_str().unwrap().to_string());
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    // First client (unused) was evicted, keeping total at MAX_CLIENTS
    assert!(!m.has_client(&cids[0]));
    assert_eq!(m.client_count(), 2);
    // Give active tokens to both remaining clients -> 4th registration returns 429
    m.issue_tokens(&cids[1]);
    m.issue_tokens(&cids[2]);
    let res = m.handle(&post_json("/register", json!({"redirect_uris": ["https://app3.example/cb"]})));
    mcp::MAX_CLIENTS.store(old, Ordering::SeqCst);
    assert_eq!((res.status, res.text_body().as_str()), (429, "{\"error\":\"too_many_clients\"}"));
    // a public client gets no secret
    let res = m.handle(&post_json("/register", json!({"redirect_uris": ["https://pub.example/cb"], "token_endpoint_auth_method": "none"})));
    assert!(body_json(&res).get("client_secret").is_none());
}

#[test]
fn authorize_pkce_lockout_and_refresh_rotation() {
    let _g = serial();
    let tmp = TempDir::new("mcp_auth");
    let (m, _) = server(&tmp, 1);
    let res = m.handle(&post_json("/register", json!({"redirect_uris": ["https://gemini.google.com/cb"], "token_endpoint_auth_method": "none"})));
    assert_eq!(res.status, 201);
    let cid = body_json(&res)["client_id"].as_str().unwrap().to_string();
    let verifier = "verifier-secret-string-0123456789";
    let challenge = crypto::base64url(&crypto::sha256(verifier.as_bytes()));
    let base = [
        ("response_type", "code"),
        ("client_id", cid.as_str()),
        ("redirect_uri", "https://gemini.google.com/cb"),
        ("state", "st&1"),
        ("code_challenge", challenge.as_str()),
    ];
    let query = |method: &str| -> String {
        let mut q: Vec<String> = base.iter().map(|(k, v)| format!("{k}={}", mcp::quote_plus(v))).collect();
        q.push(format!("code_challenge_method={method}"));
        q.join("&")
    };
    // Reject plain PKCE; the S256 page carries the request in hidden fields
    let res = m.handle(&Request::new("GET", &format!("/authorize?{}", query("plain"))));
    assert_eq!((res.status, res.text_body().as_str()), (400, "Only S256 PKCE is supported."));
    let page = m.handle(&Request::new("GET", &format!("/authorize?{}", query("S256"))));
    assert_eq!(page.status, 200);
    assert_eq!(page.header("content-type"), "text/html; charset=utf-8");
    assert!(page.text_body().contains("<input type=\"hidden\" name=\"state\" value=\"st&amp;1\">"));
    assert!(page.text_body().contains("name=\"pairing_code\""));
    assert_eq!(m.handle(&Request::new("GET", "/authorize?response_type=code&client_id=nope&redirect_uri=https://x.example/")).status, 400);

    let attempt = |pin: &str| {
        let mut fields: Vec<(&str, &str)> = base.to_vec();
        fields.push(("code_challenge_method", "S256"));
        fields.push(("pairing_code", pin));
        m.handle(&form("/authorize", &fields))
    };
    // Valid authorize + wrong pin lockout after 5 failures
    for _ in 0..5 {
        let res = attempt("00000000");
        assert_eq!(res.status, 200);
        assert!(res.text_body().contains("<p class=\"e\">"));
    }
    // 6th try even with correct pin is locked out
    assert_eq!(attempt("12345678").status, 200);

    // A fresh server (no failures) completes authorization -> token -> refresh rotation
    let (m, _) = server(&tmp, 1);
    let attempt = |pin: &str| {
        let mut fields: Vec<(&str, &str)> = base.to_vec();
        fields.push(("code_challenge_method", "S256"));
        fields.push(("pairing_code", pin));
        m.handle(&form("/authorize", &fields))
    };
    let res = attempt(" 12345678 ");
    assert_eq!(res.status, 302);
    let location = res.header("location").to_string();
    assert!(location.starts_with("https://gemini.google.com/cb?code="), "{location}");
    assert!(location.ends_with("&iss=https%3A%2F%2Fbox.ts.net%3A10000&state=st%261"), "{location}");
    let code = location.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();

    let token = |fields: &[(&str, &str)]| m.handle(&form("/token", fields));
    assert_eq!(token(&[("grant_type", "authorization_code"), ("code", &code)]).status, 401);
    let res = token(&[
        ("grant_type", "authorization_code"),
        ("client_id", &cid),
        ("code", &code),
        ("redirect_uri", "https://gemini.google.com/cb"),
        ("code_verifier", verifier),
    ]);
    assert_eq!(res.status, 200);
    assert_eq!(res.header("cache-control"), "no-store");
    let toks = body_json(&res);
    assert_eq!(toks["token_type"], "Bearer");
    assert_eq!(toks["expires_in"], 86400);
    assert!(m.token_valid(toks["access_token"].as_str().unwrap(), "access"));
    // the code works once
    assert_eq!(token(&[("grant_type", "authorization_code"), ("client_id", &cid), ("code", &code), ("code_verifier", verifier)]).status, 400);

    // Rotate refresh token
    let refresh = toks["refresh_token"].as_str().unwrap();
    let res = token(&[("grant_type", "refresh_token"), ("client_id", &cid), ("refresh_token", refresh)]);
    assert_eq!(res.status, 200);
    assert_ne!(body_json(&res)["refresh_token"], toks["refresh_token"]);
    // Old refresh token must no longer work
    assert_eq!(token(&[("grant_type", "refresh_token"), ("client_id", &cid), ("refresh_token", refresh)]).status, 400);
    assert_eq!(token(&[("grant_type", "password"), ("client_id", &cid)]).text_body(), "{\"error\":\"unsupported_grant_type\"}");

    // a wrong verifier is refused
    let location = attempt("12345678").header("location").to_string();
    let code = location.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
    let res = token(&[("grant_type", "authorization_code"), ("client_id", &cid), ("code", &code), ("code_verifier", "wrong")]);
    assert_eq!(res.text_body(), "{\"error\":\"invalid_grant\",\"error_description\":\"PKCE\"}");

    // the store on disk keeps hashes only, in the layout the Python server writes
    let saved = std::fs::read_to_string(tmp.join("mcp_auth.json")).unwrap();
    assert!(saved.starts_with("{\n \"clients\": {\n  \"gf-"), "{}", &saved[..60]);
    assert!(!saved.contains(toks["access_token"].as_str().unwrap()));
    assert!(saved.contains(&crypto::sha256_hex(toks["access_token"].as_str().unwrap())));
}

#[test]
fn static_client_uses_its_secret() {
    let _g = serial();
    let tmp = TempDir::new("mcp_static");
    let (m, _) = server(&tmp, 1);
    let refresh = m.issue_tokens("geminiforge")["refresh_token"].as_str().unwrap().to_string();
    let wrong = m.handle(&form("/token", &[("grant_type", "refresh_token"), ("client_id", "geminiforge"), ("client_secret", "nope"), ("refresh_token", &refresh)]));
    assert_eq!((wrong.status, wrong.text_body().as_str()), (401, "{\"error\":\"invalid_client\"}"));
    // HTTP Basic works too: base64("geminiforge:test-secret")
    let basic = m.handle(
        &form("/token", &[("grant_type", "refresh_token"), ("refresh_token", &refresh)])
            .with_header("authorization", "Basic Z2VtaW5pZm9yZ2U6dGVzdC1zZWNyZXQ="),
    );
    assert_eq!(basic.status, 200);
    // any https redirect is fine for the configured client, and it needs no PKCE
    let page = m.handle(&Request::new("GET", "/authorize?response_type=code&client_id=geminiforge&redirect_uri=https://any.example/x"));
    assert_eq!(page.status, 200);
    assert_eq!(m.handle(&Request::new("GET", "/authorize?response_type=code&client_id=geminiforge&redirect_uri=http://any.example/x")).status, 400);
}

#[test]
fn metadata_and_plain_routes() {
    let _g = serial();
    let tmp = TempDir::new("mcp_meta");
    let (m, _) = server(&tmp, 1);
    let res = m.handle(&Request::new("GET", "/.well-known/oauth-protected-resource"));
    assert_eq!(
        res.text_body(),
        "{\"resource\":\"https://box.ts.net:10000/mcp\",\"authorization_servers\":[\"https://box.ts.net:10000\"],\"bearer_methods_supported\":[\"header\"],\"scopes_supported\":[\"geminiforge\"]}"
    );
    let meta = body_json(&m.handle(&Request::new("GET", "/.well-known/oauth-authorization-server/mcp")));
    assert_eq!(meta["token_endpoint"], "https://box.ts.net:10000/token");
    assert_eq!(meta["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(m.handle(&Request::new("GET", "/")).text_body(), "GeminiForge MCP server. Endpoint: /mcp");
    assert!(m.handle(&Request::new("HEAD", "/")).body.is_empty());
    assert_eq!(m.handle(&Request::new("GET", "/nope")).status, 404);
    let res = m.handle(&Request::new("GET", "/register"));
    assert_eq!((res.status, res.header("allow")), (405, "POST"));
    // without a configured public address the Host header is used
    m.set_cfg("public_url", "");
    let res = m.handle(&Request::new("GET", "/.well-known/openid-configuration").with_header("host", "mybox.ts.net:10000"));
    assert_eq!(body_json(&res)["issuer"], "https://mybox.ts.net:10000");
}

fn raw(port: u16, bytes: &[u8]) -> String {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    s.write_all(bytes).unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn mcp_auth_checked_before_body_read_and_size_capped() {
    let _g = serial();
    let tmp = TempDir::new("mcp_sock");
    let (m, _) = server(&tmp, 1);
    m.set_cfg("public_url", "");
    let token = access_token(&m);
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let m = Arc::new(m);
    let served = m.clone();
    std::thread::spawn(move || mcp::serve(served, listener));

    // No token: answered at once although the announced body never arrives
    let started = std::time::Instant::now();
    let res = raw(port, b"POST /mcp HTTP/1.1\r\nHost: mybox.ts.net:10000\r\nContent-Type: application/json\r\nContent-Length: 100000\r\n\r\n");
    assert!(res.starts_with("HTTP/1.1 401 "), "{res}");
    assert!(started.elapsed().as_secs_f64() < 5.0);
    assert!(res.to_lowercase().contains("www-authenticate: bearer resource_metadata=\"https://mybox.ts.net:10000/.well-known/oauth-protected-resource\", scope=\"geminiforge\""), "{res}");
    assert!(res.ends_with("{\"error\": \"invalid_token\"}"));

    let old = mcp::MAX_MCP_BODY.swap(16, Ordering::SeqCst);
    let big = raw(
        port,
        format!("POST /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: 20\r\n\r\n01234567890123456789").as_bytes(),
    );
    mcp::MAX_MCP_BODY.store(old, Ordering::SeqCst);
    assert!(big.starts_with("HTTP/1.1 413 "), "{big}");
    assert!(big.ends_with("{\"error\": \"payload_too_large\"}"));

    // a real request over the socket, sent in chunks
    let body = "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}";
    let ping = raw(
        port,
        format!(
            "POST /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    );
    assert!(ping.starts_with("HTTP/1.1 200 "), "{ping}");
    assert!(ping.to_lowercase().contains("content-type: text/event-stream"));
    assert!(ping.ends_with("event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\r\n\r\n"), "{ping}");
    assert!(raw(port, b"GET /mcp/ HTTP/1.1\r\nHost: x\r\n\r\n").starts_with("HTTP/1.1 401 "));
}

#[test]
fn protocol_answers() {
    let _g = serial();
    let tmp = TempDir::new("mcp_proto");
    let (m, _) = server(&tmp, 1);
    let token = access_token(&m);
    let init = |version: &str| {
        rpc(&call(&m, &token, "initialize", json!({"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}})))
    };
    let res = init("2025-06-18");
    assert_eq!(res["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(res["result"]["serverInfo"], json!({"name": "GeminiForge", "title": "GeminiForge", "version": "1.3.0"}));
    assert_eq!(
        res["result"]["capabilities"],
        json!({"prompts": {"listChanged": false}, "resources": {"listChanged": false, "subscribe": false}, "tools": {"listChanged": false}})
    );
    let instructions = res["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains(if cfg!(windows) { "click_element" } else { "read_file" }), "{instructions}");
    assert_eq!(init("2024-11-05")["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init("1999-01-01")["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(rpc(&call(&m, &token, "initialize", json!({})))["error"]["code"], -32602);

    let res = call(&m, &token, "ping", Value::Null);
    assert_eq!(res.header("content-type"), "text/event-stream");
    assert_eq!(res.header("cache-control"), "no-cache, no-transform");
    assert_eq!(res.text_body(), "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\r\n\r\n");
    assert_eq!(rpc(&call(&m, &token, "resources/list", Value::Null))["result"], json!({"resources": []}));
    assert_eq!(rpc(&call(&m, &token, "prompts/list", Value::Null))["result"], json!({"prompts": []}));
    assert_eq!(rpc(&call(&m, &token, "nope/nope", Value::Null))["error"], json!({"code": -32601, "message": "Method not found", "data": "nope/nope"}));

    let bearer = format!("Bearer {token}");
    let with = |r: Request| r.with_header("authorization", &bearer);
    // a notification has nothing to answer
    let note = m.handle(&with(post_json("/mcp", json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))).with_header("accept", "application/json, text/event-stream"));
    assert_eq!((note.status, note.body.len()), (202, 0));
    // the client must accept both answer forms
    let res = m.handle(&with(post_json("/mcp", json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))));
    assert_eq!(res.status, 406);
    assert_eq!(
        res.text_body(),
        "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32600,\"message\":\"Not Acceptable: Client must accept both application/json and text/event-stream\"}}"
    );
    let res = m.handle(&with(Request::new("POST", "/mcp").with_header("accept", "application/json, text/event-stream").with_header("content-type", "text/plain").with_body(b"{}")));
    assert_eq!((res.status, res.text_body().as_str()), (400, "Invalid Content-Type header"));
    let res = m.handle(&with(Request::new("POST", "/mcp").with_header("accept", "application/json, text/event-stream").with_header("content-type", "application/json").with_body(b"{nope")));
    assert_eq!((res.status, body_json(&res)["error"]["code"].as_i64()), (400, Some(-32700)));
    assert_eq!(m.handle(&with(Request::new("DELETE", "/mcp"))).status, 405);
    let stream = m.handle(&with(Request::new("GET", "/mcp")).with_header("accept", "text/event-stream"));
    assert!(stream.stream && stream.status == 200);
    let moved = m.handle(&with(post_json("/mcp/", json!({}))).with_header("host", "h:1"));
    assert_eq!((moved.status, moved.header("location")), (307, "http://h:1/mcp"));
    // no token, wrong kind of token
    assert_eq!(m.handle(&post_json("/mcp", json!({}))).status, 401);
    let refresh = m.issue_tokens("geminiforge")["refresh_token"].as_str().unwrap().to_string();
    assert_eq!(call(&m, &refresh, "ping", Value::Null).status, 401);
}

#[test]
#[cfg(not(windows))]
fn mcp_lists_only_the_tools_this_system_has() {
    let _g = serial();
    let tmp = TempDir::new("mcp_unix_list");
    let (m, _) = server(&tmp, 1);
    let token = access_token(&m);
    let listed = rpc(&call(&m, &token, "tools/list", Value::Null));
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, geminiforge::tools::tool_names());
    assert_eq!(names.len(), 20);
    assert!(!names.contains(&"click") && names.contains(&"screenshot") && names.contains(&"read_files"));
    let run = listed["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "run").unwrap();
    assert!(run["description"].as_str().unwrap().starts_with("Run a command in bash"));
    let res = rpc(&call(&m, &token, "tools/call", json!({"name": "click", "arguments": {"x": 1, "y": 1}})))["result"].clone();
    assert_eq!(res["isError"], true);
    assert!(res["content"][0]["text"].as_str().unwrap().contains("click is not available on "), "{res}");
}

#[test]
#[cfg(windows)]
fn mcp_tool_schemas_for_ui_and_scroll_tools() {
    let _g = serial();
    let tmp = TempDir::new("mcp_schema");
    let (m, _) = server(&tmp, 1);
    let token = access_token(&m);
    let listed = rpc(&call(&m, &token, "tools/list", Value::Null));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 40);
    let expected: [(&str, &[&str]); 11] = [
        ("ui_elements", &["window", "filter", "max", "include_text", "window_id", "include_offscreen", "offset"]),
        ("read_window", &["window", "window_id", "include_offscreen", "offset", "max_chars"]),
        ("type_element", &["text", "id", "name", "window_id", "clear", "enter", "restore_focus", "screenshot"]),
        ("scroll", &["amount", "x", "y", "window_id", "percent", "screenshot"]),
        ("key", &["keys", "window_id", "restore_focus", "screenshot"]),
        ("type", &["text", "enter", "window_id", "restore_focus", "screenshot"]),
        ("screenshot", &["grid", "monitor", "window", "window_id"]),
        ("ocr_window", &["window", "window_id", "filter", "overlay", "max"]),
        ("click_text", &["text", "id", "window_id", "button", "clicks", "background", "cursor_jump", "restore_focus", "capture_id", "screenshot"]),
        ("wait_for", &["window", "window_id", "text", "element", "gone", "use_ocr", "timeout", "interval"]),
        ("gui_batch", &["steps", "window_id", "restore_focus", "screenshot"]),
    ];
    let by_name = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap();
    for (name, props) in expected {
        let schema = &by_name(name)["inputSchema"];
        let got: Vec<&str> = schema["properties"].as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(got, props, "{name}");
        let raw = schema.to_string();
        assert!(!raw.contains("anyOf") && !raw.contains("\"title\""), "{name}");
    }
    assert_eq!(by_name("type_element")["inputSchema"]["required"], json!(["text"]));
    assert_eq!(by_name("gui_batch")["inputSchema"]["required"], json!(["steps"]));
    assert!(by_name("clipboard_get")["inputSchema"].get("required").is_none());
    // read-only tools say so; the rest are marked destructive
    assert_eq!(by_name("read_file")["annotations"], json!({"destructiveHint": false, "readOnlyHint": true}));
    assert_eq!(by_name("write_file")["annotations"], json!({"destructiveHint": true, "readOnlyHint": false}));
    // web_confirm off: every tool is reported read-only, so Gemini never asks
    m.set_cfg("web_confirm", "off");
    let listed = rpc(&call(&m, &token, "tools/list", Value::Null));
    assert!(listed["result"]["tools"].as_array().unwrap().iter().all(|t| t["annotations"]["readOnlyHint"] == true));
}

#[test]
fn tool_error_survives_as_execution_error_and_denied_action_never_reaches_the_tool() {
    let _g = serial();
    let tmp = TempDir::new("mcp_call");
    let (m, shown) = server(&tmp, 1);
    let token = access_token(&m);
    let run = |name: &str, args: Value| rpc(&call(&m, &token, "tools/call", json!({"name": name, "arguments": args})))["result"].clone();
    // read-only tools run without asking
    let ok = run("wait", json!({"seconds": 0}));
    assert_eq!(ok, json!({"content": [{"text": "Waited 0.0 seconds", "type": "text"}], "isError": false}));
    let missing = run("read_file", json!({"path": tmp.join("nope.txt").to_string_lossy()}));
    assert_eq!(missing["isError"], true);
    assert!(missing["content"][0]["text"].as_str().unwrap().starts_with("Error executing tool read_file: File not found: "));
    assert_eq!(run("nope", json!({}))["content"][0]["text"], "Unknown tool: nope");
    let invalid = run("read_file", json!({}));
    assert!(invalid["content"][0]["text"].as_str().unwrap().starts_with("Error executing tool read_file: 1 validation error for read_fileArguments\npath\n  Field required"));
    assert_eq!(shown.load(Ordering::SeqCst), 0);

    // a tool that changes things is asked about; "deny" stops it before it runs
    let before = state::cwd();
    let target = tmp.join("denied.txt");
    let denied = run("write_file", json!({"path": target.to_string_lossy(), "content": "x"}));
    assert_eq!(
        denied["content"][0]["text"],
        "Error executing tool write_file: The user denied this action on the computer. Stop; do not use other tools to get around the denial."
    );
    assert_eq!(denied["isError"], true);
    assert!(!target.exists());
    assert_eq!(shown.load(Ordering::SeqCst), 1);
    assert_eq!(state::cwd(), before);
    // approval "none": nothing is asked
    m.set_cfg("approval", "none");
    geminiforge::backups::set_root(&tmp.join("backups"));
    let written = run("write_file", json!({"path": target.to_string_lossy(), "content": "x"}));
    assert_eq!(written["isError"], false, "{written}");
    assert!(target.exists());
    assert_eq!(shown.load(Ordering::SeqCst), 1);
}

#[test]
fn approve_shows_one_popup_at_a_time_and_remembers_a_timed_grant() {
    let _g = serial();
    let tmp = TempDir::new("mcp_approve");
    let (m, shown) = server(&tmp, 2);
    let m = Arc::new(m);
    let args = json!({"path": "x.txt", "content": "a".repeat(45000)}).as_object().cloned().unwrap();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (m, args) = (m.clone(), args.clone());
            std::thread::spawn(move || m.approve("write_file", &args))
        })
        .collect();
    let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    // "allow for 30 minutes" from the first popup covers the second call
    assert_eq!(results, vec![true, true]);
    assert_eq!(shown.load(Ordering::SeqCst), 1);
    // another tool is asked about separately; read-only tools never are
    assert!(m.approve("run", &args));
    assert_eq!(shown.load(Ordering::SeqCst), 2);
    assert!(m.approve("read_file", &args));
    assert_eq!(shown.load(Ordering::SeqCst), 2);
    // approval "all" asks even for read-only tools
    m.set_cfg("approval", "all");
    assert!(m.approve("read_file", &args));
    assert_eq!(shown.load(Ordering::SeqCst), 3);

    let (deny, _) = server(&tmp, 1);
    assert!(!deny.approve("run", &args));
    let (broken, _) = server(&tmp, 9);
    assert!(!broken.approve("run", &args));
}

#[test]
fn rpc_log_skips_pings_and_access_log_rotates() {
    let _g = serial();
    let tmp = TempDir::new("mcp_log");
    let (m, _) = server(&tmp, 1);
    let log = tmp.join("mcp_access.log");
    m.rpc_log("←", b"");
    m.rpc_log("←", b": ping - 2026-11-19\n\n: ping - 2026-11-19\n\n");
    assert!(!log.exists());
    m.rpc_log("→", b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"read_file\"}}");
    m.rpc_log("←", b"event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"content\":[],\"isError\":false}}\r\n\r\n");
    m.rpc_log("←", b"not json at all");
    let text = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].ends_with("  → tools/call read_file"), "{}", lines[0]);
    assert!(lines[1].ends_with("  ← result content,isError"), "{}", lines[1]);
    assert!(lines[2].ends_with("  ← (raw) not json at all"), "{}", lines[2]);

    let old = mcp::MAX_LOG_BYTES.swap(40, Ordering::SeqCst);
    m.rpc_log("→", b"{\"jsonrpc\":\"2.0\",\"method\":\"second-line\"}");
    mcp::MAX_LOG_BYTES.store(old, Ordering::SeqCst);
    assert!(tmp.join("mcp_access.log.1").exists());
    assert!(std::fs::read_to_string(&log).unwrap().contains("second-line"));
    // every request is noted with its status
    m.handle(&Request::new("GET", "/nope?x=1").with_header("user-agent", "probe").with_header("x-forwarded-for", "1.2.3.4"));
    assert!(std::fs::read_to_string(&log).unwrap().lines().last().unwrap().ends_with(" GET /nope?x=1 -> 404 [1.2.3.4] probe"));
}

#[test]
fn config_defaults_are_written_once() {
    let _g = serial();
    let tmp = TempDir::new("mcp_cfg");
    let m = Mcp::with_approver(tmp.path(), 18767, Box::new(|_, _| 1), false);
    let saved = std::fs::read_to_string(tmp.join("mcp_config.json")).unwrap();
    assert!(saved.starts_with("{\n  \"public_url\": \"\",\n  \"pairing_code\": \""), "{saved}");
    assert_eq!(m.cfg("pairing_code").len(), 8);
    assert!(m.cfg("pairing_code").chars().all(|c| c.is_ascii_digit()));
    assert_eq!((m.cfg("client_id").as_str(), m.cfg("approval").as_str(), m.cfg("web_confirm").as_str()), ("geminiforge", "readonly", "on"));
    assert_eq!(m.cfg("client_secret").len(), 43);
    assert_eq!(m.cfg("tunnel"), "tailscale");
    // a complete config is left exactly as it is
    let again = Mcp::with_approver(tmp.path(), 18767, Box::new(|_, _| 1), false);
    assert_eq!(std::fs::read_to_string(tmp.join("mcp_config.json")).unwrap(), saved);
    assert_eq!(again.cfg("pairing_code"), m.cfg("pairing_code"));
}

#[test]
fn readonly_sets_match_between_mcp_and_extension() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut mine: Vec<&str> = mcp::READ_ONLY.to_vec();
    mine.sort();
    let quoted = |text: &str| -> Vec<String> {
        let mut out: Vec<String> = text.split('"').skip(1).step_by(2).map(|s| s.to_string()).collect();
        out.sort();
        out
    };
    if let Ok(js) = std::fs::read_to_string(root.join("extension").join("content.js")) {
        let start = js.find("const READ_ONLY = new Set([").expect("READ_ONLY in content.js");
        let block = &js[start..start + js[start..].find("]);").unwrap()];
        assert_eq!(quoted(&block["const READ_ONLY = new Set([".len()..]), mine);
    }
}
