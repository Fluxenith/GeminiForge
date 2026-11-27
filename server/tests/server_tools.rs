//! File, backup, command and HTTP behaviour (the Rust side of
//! server/test_server_tools.py). Temporary folders and loopback only.

mod common;

use common::{backup_id, call, err, serial, show, TempDir};
use geminiforge::py::Error;
use geminiforge::state::{self, MAX_HTTP_BODY, MAX_READ_BYTES, RUN_WAIT_MS};
use geminiforge::{backups, http, proc_tools};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::Ordering;

/// The same command for PowerShell (Windows) and bash (macOS, Linux).
fn sh(windows: &'static str, unix: &'static str) -> &'static str {
    if cfg!(windows) {
        windows
    } else {
        unix
    }
}

fn sandbox(tag: &str) -> TempDir {
    let tmp = TempDir::new(tag);
    state::with(|s| s.cwd = show(tmp.path()));
    backups::set_root(&tmp.join("backups"));
    tmp
}

#[test]
fn write_edit_and_backup_restore_roundtrip() {
    let _g = serial();
    let tmp = sandbox("roundtrip");
    let f = tmp.join("sample.txt");
    fs::write(&f, b"line1\r\nline2\r\nline3\r\n").unwrap();

    // Empty old and binary files must be rejected
    assert!(call("edit_file", json!({"path": show(&f), "old": "", "new": "x"})).unwrap_err().is_tool());
    let bin = tmp.join("data.bin");
    fs::write(&bin, b"\x00\x01\x02\x03").unwrap();
    assert!(call("edit_file", json!({"path": show(&bin), "old": "a", "new": "b"})).unwrap_err().is_tool());

    // CRLF tolerance must not double CR (\r\r\n)
    let res = call("edit_file", json!({"path": show(&f), "old": "line1\nline2", "new": "alpha\nbeta"})).unwrap();
    assert!(res.contains("1 replacement(s)"), "{res}");
    assert_eq!(fs::read(&f).unwrap(), b"alpha\r\nbeta\r\nline3\r\n");

    // Restore back to original via backup_id
    call("restore_backup", json!({"backup_id": backup_id(&res)})).unwrap();
    assert_eq!(fs::read(&f).unwrap(), b"line1\r\nline2\r\nline3\r\n");

    // Write a brand-new file then restore its "did not exist" backup -> deletes it
    let new_f = tmp.join("created.txt");
    let wres = call("write_file", json!({"path": show(&new_f), "content": "hello"})).unwrap();
    assert!(wres.contains("(5 characters, 1 lines)"), "{wres}");
    assert!(new_f.is_file());
    call("restore_backup", json!({"backup_id": backup_id(&wres)})).unwrap();
    assert!(!new_f.exists());

    // write_file keeps the text byte for byte and creates missing folders
    let nested = tmp.join("a").join("b").join("c.txt");
    call("write_file", json!({"path": show(&nested), "content": "x\r\ny\nテスト"})).unwrap();
    assert_eq!(fs::read(&nested).unwrap(), "x\r\ny\nテスト".as_bytes());
    assert!(err("write_file", json!({"path": show(tmp.path()), "content": "x"})).starts_with("This is a folder, not a file:"));
    assert_eq!(
        err("write_file", json!({"path": show(&nested), "content": 5})),
        "Invalid arguments: data must be str, not int"
    );

    // several matches need replace_all
    let dup = tmp.join("dup.txt");
    fs::write(&dup, "foo bar foo").unwrap();
    assert_eq!(
        err("edit_file", json!({"path": show(&dup), "old": "foo", "new": "X"})),
        "`old` appears 2 times; add more context to make it unique, or set replace_all=true."
    );
    let all = call("edit_file", json!({"path": show(&dup), "old": "foo", "new": "X", "replace_all": true})).unwrap();
    assert!(all.contains("(2 replacement(s))"), "{all}");
    assert_eq!(fs::read_to_string(&dup).unwrap(), "X bar X");

    let listing = call("list_backups", json!({"path": "dup.txt"})).unwrap();
    assert!(listing.contains("edit_file") && listing.contains("dup.txt"), "{listing}");
    assert!(err("restore_backup", json!({"backup_id": "abc"})).contains("abc"));
}

#[test]
fn restore_missing_payload_fails_before_undo_snapshot() {
    let _g = serial();
    let tmp = sandbox("payload");
    let target = tmp.join("target.txt");
    fs::write(&target, "keep").unwrap();
    let (bid, _) = backups::snapshot(&target, "write_file").unwrap();
    fs::remove_file(backups::root().join(&bid).join("file")).unwrap();
    let count = || fs::read_dir(backups::root()).unwrap().count();
    let before = count();
    assert!(matches!(backups::restore(&bid), Err(Error::Exc(kind, _)) if kind == "ValueError"));
    assert_eq!(count(), before);
}

#[test]
fn prune_ignores_unrelated_directories_in_backups_root() {
    let _g = serial();
    let _tmp = sandbox("prune");
    let root = backups::root();
    let unrelated = root.join("manual-notes");
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(unrelated.join("keep.txt"), "ok").unwrap();
    let old_entry = root.join("20200101-000000-001");
    fs::create_dir_all(&old_entry).unwrap();
    fs::write(old_entry.join("meta.json"), "{}").unwrap();
    backups::prune();
    assert!(unrelated.exists());
    assert!(!old_entry.exists());
}

#[test]
fn read_file_normal_and_streaming_large_file() {
    let _g = serial();
    let tmp = sandbox("read");
    let f = tmp.join("many.txt");
    let text: Vec<String> = (1..=25).map(|i| format!("row-{i}")).collect();
    fs::write(&f, text.join("\n")).unwrap();
    let normal = call("read_file", json!({"path": show(&f), "offset": 2, "limit": 3})).unwrap();
    assert!(normal.contains("    2\trow-2"), "{normal}");
    assert!(normal.contains("    4\trow-4"));
    assert!(!normal.contains("\trow-5"));
    assert!(normal.contains("25 lines total; continue with offset=5"));

    let old = MAX_READ_BYTES.swap(20, Ordering::SeqCst);
    let streamed = call("read_file", json!({"path": show(&f), "offset": 5, "limit": 4}));
    MAX_READ_BYTES.store(old, Ordering::SeqCst);
    let streamed = streamed.unwrap();
    assert!(streamed.contains("streamed partial read"), "{streamed}");
    assert!(streamed.contains("5\trow-5"));
    assert!(streamed.contains("8\trow-8"));
    assert!(!streamed.contains("9\trow-9"));

    assert!(err("read_file", json!({"path": show(&tmp.join("nope.txt"))})).starts_with("File not found:"));
    let bin = tmp.join("data.bin");
    fs::write(&bin, b"\x00\x01").unwrap();
    assert!(err("read_file", json!({"path": show(&bin)})).starts_with("Binary file, cannot read as text:"));
}

#[test]
fn search_and_find_files_skip_dirs_and_work_under_build_parent() {
    let _g = serial();
    let tmp = sandbox("search");
    let build_parent = tmp.join("build").join("myproj");
    fs::create_dir_all(build_parent.join("sub")).unwrap();
    fs::create_dir_all(build_parent.join(".venv").join("lib")).unwrap();
    fs::write(build_parent.join("app.py"), "needle_here = 1\n").unwrap();
    fs::write(build_parent.join("sub").join("mod.py"), "needle_here = 2\n").unwrap();
    fs::write(build_parent.join(".venv").join("lib").join("ignored.py"), "needle_here = 3\n").unwrap();
    let root = show(&build_parent);

    let found = call("find_files", json!({"pattern": "*.py", "path": root})).unwrap();
    assert!(found.contains("app.py") && found.contains("mod.py"), "{found}");
    assert!(!found.contains("ignored.py"));

    let deep = call("find_files", json!({"pattern": "sub/*.py", "path": root})).unwrap();
    assert!(deep.contains("mod.py") && !deep.contains("app.py"), "{deep}");

    let hits = call("search", json!({"pattern": "needle_here", "path": root, "glob": "**/*.py"})).unwrap();
    assert!(hits.contains("app.py:1:") && hits.contains("mod.py:1:"), "{hits}");
    assert!(!hits.contains("ignored.py"));
    assert_eq!(call("search", json!({"pattern": "NEEDLE", "path": root, "ignore_case": false})).unwrap(), "No matches found");
    assert!(err("search", json!({"pattern": "(", "path": root})).starts_with("Invalid regular expression:"));

    let missing = show(&build_parent.join("nonexistent"));
    assert!(call("search", json!({"pattern": "x", "path": missing})).unwrap_err().is_tool());
    assert!(call("find_files", json!({"pattern": "*.py", "path": missing})).unwrap_err().is_tool());
}

#[test]
fn cd_and_list_dir_follow_the_current_folder() {
    let _g = serial();
    let tmp = sandbox("cd");
    fs::create_dir_all(tmp.join("Sub")).unwrap();
    fs::write(tmp.join("b.txt"), "12345").unwrap();
    fs::write(tmp.join("A.txt"), "").unwrap();
    // Windows file names ignore case and the folder is reported with its real case
    let sub = if cfg!(windows) { "sub" } else { "Sub" };
    assert_eq!(call("cd", json!({"path": sub})).unwrap(), format!("Current folder: {}", show(&tmp.join("Sub"))));
    assert_eq!(call("cd", json!({"path": ".."})).unwrap(), format!("Current folder: {}", show(tmp.path())));
    assert!(err("cd", json!({"path": "nope"})).starts_with("Folder not found:"));
    let listing = call("list_dir", json!({})).unwrap();
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines[0], show(tmp.path()));
    // folders first, then names without regard to case
    assert_eq!(lines[1..], ["  Sub/", "  A.txt  (0 B)", "  b.txt  (5 B)"]);
    assert_eq!(
        err("list_dir", json!({"depth": "abc"})),
        "ValueError: invalid literal for int() with base 10: 'abc'"
    );
}

#[test]
fn click_and_key_reject_invalid_inputs() {
    let _g = serial();
    assert!(err("nope", json!({})).starts_with("Unknown tool: nope. Available: cd, list_dir, read_file,"));
    if !cfg!(windows) {
        let msg = err("click", json!({"x": 10, "y": 10}));
        assert!(msg.starts_with("click is not available on "), "{msg}");
        assert!(msg.contains("read_files, write_file") && msg.contains("screenshot"), "{msg}");
        assert!(!err("nope", json!({})).contains("click"));
        return;
    }
    assert_eq!(
        call("click", json!({"x": 10, "y": 10, "button": "sideways"})).unwrap_err(),
        Error::tool("Invalid mouse button: sideways")
    );
    assert_eq!(call("key", json!({"keys": " + "})).unwrap_err(), Error::tool("Provide at least one key."));
    assert_eq!(
        err("click", json!({"x": 10})),
        "Invalid arguments: t_click() missing 1 required positional argument: 'y'"
    );
}

#[test]
fn run_fast_timeout_and_background_job_lifecycle() {
    let _g = serial();
    let tmp = sandbox("run");
    proc_tools::clear_jobs();
    assert!(call("run", json!({"command": sh("Write-Output ok", "echo ok"), "cwd": show(&tmp.join("missing"))})).unwrap_err().is_tool());

    let out = call("run", json!({"command": sh("Write-Output \"hello_utf8_テスト\"", "echo \"hello_utf8_テスト\""), "timeout": 30})).unwrap();
    assert!(out.contains("hello_utf8_テスト"), "{out}");
    assert!(out.contains("[exit code 0]"));
    assert!(out.starts_with("[job 1 finished in "));

    let old = RUN_WAIT_MS.swap(5000, Ordering::SeqCst);
    let killed = call("run", json!({"command": sh("Start-Sleep -Seconds 15", "sleep 15"), "timeout": 1}));
    RUN_WAIT_MS.store(200, Ordering::SeqCst);
    let bg = call("run", json!({"command": sh("Write-Output 'bg_ready'; Start-Sleep -Seconds 20", "echo bg_ready; sleep 20"), "timeout": 15}));
    RUN_WAIT_MS.store(old, Ordering::SeqCst);
    assert!(killed.unwrap().contains("stopped after timeout"));
    let bg = bg.unwrap();
    assert!(bg.contains("still running in the background (job 3,"), "{bg}");
    assert!(call("job_status", json!({})).unwrap().contains("3: running"));
    let stopped = call("stop_job", json!({"id": 3})).unwrap();
    assert!(stopped.contains("stopped by stop_job"), "{stopped}");
    assert_eq!(err("job_status", json!({"id": 99})), "No job 99. Call job_status without id to list all jobs.");
    assert_eq!(err("stop_job", json!({"id": 99})), "No job 99.");
    proc_tools::clear_jobs();
    assert_eq!(call("job_status", json!({})).unwrap(), "No jobs started by run.");
}

#[test]
fn start_process_closes_parent_log_handle() {
    let _g = serial();
    let tmp = sandbox("start");
    assert!(call("start_process", json!({"command": sh("Start-Sleep -Seconds 1", "sleep 1"), "cwd": show(&tmp.join("missing"))}))
        .unwrap_err()
        .is_tool());
    let res = call("start_process", json!({"command": sh("Write-Output 'sp_ok'; Start-Sleep -Seconds 15", "echo sp_ok; sleep 15")})).unwrap();
    let pid: u32 = res.split("PID ").nth(1).unwrap().split(')').next().unwrap().parse().unwrap();
    let log = res.split("logged to ").nth(1).unwrap().split(';').next().unwrap().to_string();
    std::thread::sleep(std::time::Duration::from_millis(500));
    // the log must be openable for writing right away: the server kept no handle on it
    let opened = fs::OpenOptions::new().read(true).write(true).open(&log);
    proc_tools::kill_tree(pid);
    assert!(opened.is_ok(), "{opened:?}");
    for _ in 0..10 {
        if fs::remove_file(&log).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[test]
fn read_files_and_edit_files_take_several_files_in_one_call() {
    let _g = serial();
    let tmp = sandbox("batch");
    let (a, b) = (tmp.join("a.txt"), tmp.join("b.txt"));
    fs::write(&a, "one\ntwo\nthree\n").unwrap();
    fs::write(&b, "alpha\r\nbeta\r\n").unwrap();

    let both = call("read_files", json!({"paths": [show(&a), show(&b), show(&tmp.join("nope.txt"))], "limit": 2})).unwrap();
    assert!(both.contains("    1\tone\n    2\ttwo\n... (3 lines total; continue with offset=3)"), "{both}");
    assert!(both.contains("    1\talpha\n    2\tbeta"), "{both}");
    assert!(both.contains(&format!("] File not found: {}", show(&tmp.join("nope.txt")))), "{both}");
    assert_eq!(err("read_files", json!({"paths": []})), "Give at least one path in paths.");

    // one edit that does not apply leaves every file as it was
    let failed = err(
        "edit_files",
        json!({"edits": [{"path": show(&a), "old": "one", "new": "1"}, {"path": show(&b), "old": "gamma", "new": "3"}]}),
    );
    assert!(failed.starts_with("Edit 2 failed, so no file was changed: Text `old` not found"), "{failed}");
    assert_eq!(fs::read_to_string(&a).unwrap(), "one\ntwo\nthree\n");

    // edits to one file apply in order; a CRLF file takes LF text
    let res = call(
        "edit_files",
        json!({"edits": [
            {"path": show(&a), "old": "one", "new": "1"},
            {"path": show(&a), "old": "1\ntwo", "new": "1\n2"},
            {"path": show(&b), "old": "alpha\nbeta", "new": "x\ny"},
        ]}),
    )
    .unwrap();
    assert_eq!(res.lines().count(), 2, "{res}");
    assert!(res.contains("a.txt (2 replacement(s))"), "{res}");
    assert_eq!(fs::read_to_string(&a).unwrap(), "1\n2\nthree\n");
    assert_eq!(fs::read(&b).unwrap(), b"x\r\ny\r\n");
    // each file was backed up once, before the first edit
    call("restore_backup", json!({"backup_id": backup_id(&res)})).unwrap();
    assert_eq!(fs::read_to_string(&a).unwrap(), "one\ntwo\nthree\n");

    let unknown = err("edit_files", json!({"edits": [{"path": show(&a), "old": "x", "new": "y", "all": true}]}));
    assert!(unknown.contains("unknown key 'all'"), "{unknown}");
    assert_eq!(err("edit_files", json!({"edits": []})), "Give at least one edit in edits.");
}

#[test]
fn wait_is_capped_and_validated() {
    let _g = serial();
    let started = std::time::Instant::now();
    assert_eq!(call("wait", json!({"seconds": 0.2})).unwrap(), "Waited 0.2 seconds");
    assert!(started.elapsed().as_secs_f64() >= 0.19);
    assert_eq!(call("wait", json!({"seconds": -3})).unwrap(), "Waited -3 seconds");
    assert_eq!(err("wait", json!({"seconds": "abc"})), "ValueError: could not convert string to float: 'abc'");
}

fn request(port: u16, raw: &[u8]) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(raw).unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, head.to_string(), body.to_string())
}

fn post(port: u16, path: &str, origin: Option<&str>, body: &[u8]) -> (u16, String, String) {
    let mut head = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n");
    if let Some(o) = origin {
        head.push_str(&format!("Origin: {o}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    let mut raw = head.into_bytes();
    raw.extend_from_slice(body);
    request(port, &raw)
}

#[test]
fn http_handler_security_and_tool_call() {
    let _g = serial();
    let tmp = sandbox("http");
    let listener = http::bind(0).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || http::serve(listener));
    let ext = Some("chrome-extension://testext");

    let (status, head, body) = request(port, format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes());
    assert_eq!(status, 200);
    assert!(head.starts_with("HTTP/1.0 200 OK\r\n"), "{head}");
    assert!(head.contains("Content-Type: application/json; charset=utf-8"));
    let data: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(data["ok"], true);
    assert_eq!(data["languageSync"], true);
    assert_eq!(data["version"], "1.3.0");
    assert_eq!(data["tools"].as_array().unwrap().len(), if cfg!(windows) { 40 } else { 20 });
    // only macOS and Linux say which system they are
    assert_eq!(data.get("os").is_some(), !cfg!(windows));
    assert!(body.starts_with("{\"ok\": true, \"version\": \"1.3.0\", \"cwd\": "), "{body}");

    // a foreign Host (DNS rebinding) sees nothing
    assert_eq!(request(port, b"GET /health HTTP/1.1\r\nHost: evil.example\r\n\r\n").0, 404);
    assert_eq!(request(port, format!("GET /health?x=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).0, 404);
    let (status, _, body) = request(port, format!("OPTIONS /tool HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes());
    assert_eq!((status, body.as_str()), (403, "{\"ok\": false}"));
    assert_eq!(request(port, format!("PUT /tool HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).0, 501);

    // Reject missing or non-extension Origin
    for origin in [None, Some("https://gemini.google.com")] {
        let (status, _, body) = post(port, "/tool", origin, b"{\"tool\":\"cd\",\"args\":{}}");
        assert_eq!((status, body.as_str()), (403, "{\"ok\": false, \"error\": \"forbidden\"}"));
    }
    assert_eq!(post(port, "/other", ext, b"{}").0, 403);

    // Reject empty body / non-object JSON
    let (status, _, body) = post(port, "/tool", ext, b"");
    assert_eq!((status, body.as_str()), (400, "{\"ok\": false, \"error\": \"missing body\"}"));
    let (status, _, body) = post(port, "/tool", ext, b"[]");
    assert_eq!((status, body.as_str()), (400, "{\"ok\": false, \"error\": \"JSON body must be an object\"}"));
    let (status, _, body) = post(port, "/tool", ext, b"{nope");
    assert_eq!((status, body.as_str()), (400, "{\"ok\": false, \"error\": \"invalid JSON\"}"));

    // Oversized body returns 413
    let old = MAX_HTTP_BODY.swap(5, Ordering::SeqCst);
    let big = post(port, "/tool", ext, b"{\"tool\":\"cd\"}");
    MAX_HTTP_BODY.store(old, Ordering::SeqCst);
    assert_eq!((big.0, big.2.as_str()), (413, "{\"ok\": false, \"error\": \"request too large\"}"));

    // /language only accepts the five supported languages (nothing is saved here)
    assert_eq!(post(port, "/language", ext, b"{\"language\":\"fr\"}").0, 400);
    assert_eq!(post(port, "/language", ext, b"[1]").0, 400);

    // Valid read_file request succeeds, and the extension origin is echoed back
    let target = tmp.join("hi.txt");
    fs::write(&target, "abc\ndef").unwrap();
    let payload = json!({"tool": "read_file", "args": {"path": show(&target)}}).to_string();
    let (status, head, body) = post(port, "/tool", ext, payload.as_bytes());
    assert_eq!(status, 200);
    assert!(head.contains("Access-Control-Allow-Origin: chrome-extension://testext"), "{head}");
    let res: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(res["ok"], true);
    assert!(res["output"].as_str().unwrap().contains("    1\tabc\n    2\tdef"));
    assert!(res["image"].is_null());

    // tool errors come back as HTTP 200 with ok=false
    let (status, _, body) = post(port, "/tool", ext, b"{\"tool\":\"cd\",\"args\":[1]}");
    assert_eq!((status, body.as_str()), (200, "{\"ok\": false, \"error\": \"args must be a JSON object\"}"));
    let (status, _, body) = post(port, "/tool", ext, b"{\"tool\":\"read_file\",\"args\":{\"pth\":1}}");
    assert_eq!(status, 200);
    assert!(body.contains("Invalid arguments: t_read_file() got an unexpected keyword argument 'pth'. Did you mean 'path'?"), "{body}");
}

#[test]
fn walk_enters_junctions_and_matches_patterns_once_compiled() {
    let _g = serial();
    let tmp = sandbox("walk");
    fs::create_dir_all(tmp.join("real").join("inner")).unwrap();
    fs::create_dir_all(tmp.join("top").join("Sub")).unwrap();
    fs::write(tmp.join("real").join("inner").join("deep.py"), "needle = 1\n").unwrap();
    fs::write(tmp.join("top").join("Sub").join("Mod.PY"), "needle = 2\n").unwrap();
    fs::write(tmp.join("top").join("a.txt"), "needle = 3\n").unwrap();
    let top = show(&tmp.join("top"));

    // case does not matter, a pattern with a folder looks at the relative path
    let found = call("find_files", json!({"pattern": "*.py", "path": top})).unwrap();
    assert_eq!(found, show(&tmp.join("top").join("Sub").join("Mod.PY")));
    let found = call("find_files", json!({"pattern": "sub/*.py", "path": top})).unwrap();
    assert_eq!(found, show(&tmp.join("top").join("Sub").join("Mod.PY")));
    let found = call("find_files", json!({"pattern": "*", "path": top})).unwrap();
    assert_eq!(found.lines().count(), 3, "{found}");
    assert_eq!(call("find_files", json!({"pattern": "*.zzzz", "path": top})).unwrap(), "No matching files found");

    // os.walk walks into a junction: so does the walk here
    let junction = tmp.join("top").join("junc");
    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J", &show(&junction), &show(&tmp.join("real"))])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !made {
        eprintln!("skipped the junction part: mklink /J failed");
        return;
    }
    let found = call("find_files", json!({"pattern": "*.py", "path": top})).unwrap();
    let _ = std::process::Command::new("cmd").args(["/c", "rmdir", &show(&junction)]).output();
    assert!(found.contains(&show(&std::path::Path::new("junc").join("inner").join("deep.py"))), "{found}");
    assert!(found.contains("Mod.PY"));
    let hits = call("search", json!({"pattern": "needle", "path": top, "glob": "*.py"})).unwrap();
    assert!(hits.contains("Mod.PY:1: needle = 2"), "{hits}");
}
