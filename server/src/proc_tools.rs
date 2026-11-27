//! Command tools: run, job_status, stop_job, start_process, open, wait.

use crate::ffi;
#[cfg(windows)]
use crate::fs_tools::expandvars;
use crate::fs_tools::{resolve, show};
use crate::py::{self, Error, Res};
use crate::state::{self, MAX_JOBS, MAX_OUTPUT};
use crate::tools::{Args, ToolOut};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const PS_PREFIX: &str = "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;$OutputEncoding=[System.Text.Encoding]::UTF8;";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Killed {
    No,
    Timeout,
    Stopped,
}

pub struct Job {
    child: Mutex<Child>,
    pub pid: u32,
    pub out: String,
    pub err: String,
    pub cmd: String,
    pub cwd: String,
    pub started: f64,
    pub deadline: f64,
    killed: Mutex<Killed>,
}

impl Job {
    /// `proc.poll()`: the exit code once the process has ended.
    pub fn poll(&self) -> Option<u32> {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        child.try_wait().ok().flatten().map(exit_code)
    }

    /// `proc.wait(timeout=seconds)`, without raising: returns when the process ends or time is up.
    #[cfg(windows)]
    pub fn wait(&self, seconds: f64) {
        let handle = {
            let child = self.child.lock().unwrap_or_else(|e| e.into_inner());
            child.as_raw_handle() as isize
        };
        let ms = if seconds <= 0.0 { 0 } else { (seconds * 1000.0).ceil().min(4_000_000_000.0) as u32 };
        unsafe { ffi::WaitForSingleObject(handle, ms) };
    }

    /// `proc.wait(timeout=seconds)`, without raising: returns when the process ends or time is up.
    #[cfg(not(windows))]
    pub fn wait(&self, seconds: f64) {
        state::poll_until(seconds.max(0.0), 0.02, || self.poll().is_some());
    }

    fn killed(&self) -> Killed {
        *self.killed.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(windows)]
fn exit_code(s: std::process::ExitStatus) -> u32 {
    s.code().unwrap_or(0) as u32
}

/// A process ended by a signal reports 128 + the signal, as shells do (137 after a kill).
#[cfg(not(windows))]
fn exit_code(s: std::process::ExitStatus) -> u32 {
    use std::os::unix::process::ExitStatusExt;
    s.code().map(|c| c as u32).or_else(|| s.signal().map(|n| 128 + n as u32)).unwrap_or(0)
}

/// job id -> job
pub static JOBS: Mutex<BTreeMap<i64, Arc<Job>>> = Mutex::new(BTreeMap::new());

fn jobs() -> std::sync::MutexGuard<'static, BTreeMap<i64, Arc<Job>>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(not(windows))]
pub fn kill_tree(pid: u32) {
    ffi::kill_tree(pid);
}

#[cfg(windows)]
pub fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

fn decode_file(path: &str) -> String {
    fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

fn job_text(jid: i64, job: &Job, tail: Option<usize>) -> String {
    let mut out = decode_file(&job.out);
    let err = decode_file(&job.err);
    if let Some(t) = tail {
        if t > 0 && py::len(&out) > t {
            out = format!("… (earlier output omitted)\n{}", py::suffix(&out, t));
        }
    }
    let code = job.poll();
    let secs = (state::now() - job.started) as i64;
    let head = match code {
        None => {
            let rem = ((job.deadline - state::now()) as i64).max(0);
            format!("[job {jid} running for {secs}s; force-stopped in at most {rem}s]")
        }
        Some(code) => match job.killed() {
            Killed::Stopped => format!("[job {jid} stopped by stop_job] [exit code {code}]"),
            Killed::Timeout => format!("[job {jid} stopped after timeout] [exit code {code}]"),
            Killed::No => format!("[job {jid} finished in {secs}s] [exit code {code}]"),
        },
    };
    let mut text = format!("{head} [folder {}]\n{out}", job.cwd);
    if !py::strip(&err).is_empty() {
        text.push_str(&format!("\n[stderr]\n{}", py::suffix(&err, 4000)));
    }
    text
}

/// Mark the job as killed (unless it ended or was already marked) and kill its process tree.
fn kill_if_running(job: &Job, why: Killed, only_if_unmarked: bool) -> bool {
    let need = {
        let _all = jobs();
        let mut killed = job.killed.lock().unwrap_or_else(|e| e.into_inner());
        if job.poll().is_none() && !(only_if_unmarked && *killed != Killed::No) {
            *killed = why;
            true
        } else {
            false
        }
    };
    if need {
        kill_tree(job.pid);
    }
    need
}

fn watchdog(job: Arc<Job>) {
    job.wait((job.deadline - state::now()).max(0.0));
    if job.poll().is_none() {
        kill_if_running(&job, Killed::Timeout, true);
    }
}

#[cfg(not(windows))]
fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

#[cfg(windows)]
fn temp_dir() -> PathBuf {
    PathBuf::from(std::env::var("TEMP").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| ".".to_string()))
}

fn workdir(a: &Args) -> Res<PathBuf> {
    let cwd = a.or("cwd", Value::Null);
    let dir = if py::truthy(&cwd) { resolve(&cwd) } else { PathBuf::from(state::cwd()) };
    if !dir.is_dir() {
        return Err(Error::tool(format!("Folder not found: {}", show(&dir))));
    }
    Ok(dir)
}

fn command_text(a: &Args) -> Res<String> {
    match a.req("command") {
        #[cfg(windows)]
        Value::String(s) => Ok(format!("{PS_PREFIX}{s}")),
        #[cfg(not(windows))]
        Value::String(s) => Ok(s.clone()),
        other => Err(Error::type_err(format!(
            "can only concatenate str (not \"{}\") to str",
            py::type_name(other)
        ))),
    }
}

/// Commands run in Windows PowerShell.
#[cfg(windows)]
fn shell(ps: &str, dir: &Path) -> Command {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", ps])
        .current_dir(dir)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// A program on PATH, as a full path.
#[cfg(not(windows))]
pub fn find_program(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// Commands run in bash (sh where there is no bash). Each one leads its own
/// process group, so a timeout or stop_job ends everything it started.
#[cfg(not(windows))]
fn shell(script: &str, dir: &Path) -> Command {
    use std::os::unix::process::CommandExt;
    let program = find_program("bash").unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let mut cmd = Command::new(program);
    cmd.args(["-c", script]).current_dir(dir).stdin(Stdio::null()).process_group(0);
    cmd
}

pub fn t_run(a: &Args) -> Res<ToolOut> {
    let dir = workdir(a)?;
    let timeout = py::int(&a.or("timeout", json!(120)))?.clamp(1, 900);
    let ps = command_text(a)?;
    let command = py::py_str(a.req("command"));
    let jid = {
        let mut all = jobs();
        if all.len() >= MAX_JOBS {
            let finished: Vec<i64> = all.iter().filter(|(_, v)| v.poll().is_some()).map(|(k, _)| *k).collect();
            let drop_n = all.len() - MAX_JOBS + 1;
            for k in finished.into_iter().take(drop_n) {
                if let Some(old) = all.remove(&k) {
                    let _ = fs::remove_file(&old.out);
                    let _ = fs::remove_file(&old.err);
                }
            }
        }
        all.keys().next_back().copied().unwrap_or(0) + 1
    };
    let base = temp_dir().join(format!("geminiforge_job{jid}_{}", state::now() as i64));
    let (out, err) = (format!("{}.out.log", show(&base)), format!("{}.err.log", show(&base)));
    let child = {
        let fo = fs::File::create(&out).map_err(|e| py::os_error(&e, Some(&out)))?;
        let fe = fs::File::create(&err).map_err(|e| py::os_error(&e, Some(&err)))?;
        let mut cmd = shell(&ps, &dir);
        cmd.stdout(fo).stderr(fe);
        cmd.spawn().map_err(|e| py::os_error(&e, None))?
    };
    let now = state::now();
    let job = Arc::new(Job {
        pid: child.id(),
        child: Mutex::new(child),
        out,
        err,
        cmd: command,
        cwd: show(&dir),
        started: now,
        deadline: now + timeout as f64,
        killed: Mutex::new(Killed::No),
    });
    jobs().insert(jid, job.clone());
    let run_wait = state::run_wait();
    job.wait((timeout as f64).min(run_wait));
    if job.poll().is_none() && timeout as f64 <= run_wait {
        kill_if_running(&job, Killed::Timeout, true);
        job.wait(10.0);
    }
    if job.poll().is_none() {
        let watched = job.clone();
        std::thread::spawn(move || watchdog(watched));
        return Ok(format!(
            "{}\n\nThe command is still running in the background (job {jid}, PID {}). Use job_status {{id: {jid}, seconds: 30}} to wait for it and get the output; stop it with stop_job {{id: {jid}}}.",
            job_text(jid, &job, Some(3000)),
            job.pid
        )
        .into());
    }
    Ok(job_text(jid, &job, None).into())
}

pub fn t_job_status(a: &Args) -> Res<ToolOut> {
    let Some(id) = a.opt("id") else {
        let items: Vec<(i64, Arc<Job>)> = jobs().iter().rev().take(20).map(|(k, v)| (*k, v.clone())).collect();
        if items.is_empty() {
            return Ok("No jobs started by run.".to_string().into());
        }
        let lines: Vec<String> = items
            .iter()
            .map(|(j, v)| {
                let state = if v.poll().is_none() { "running" } else { "finished" };
                format!("{j}: {state}  {}", py::prefix(&v.cmd, 120))
            })
            .collect();
        return Ok(lines.join("\n").into());
    };
    let jid = py::int(id)?;
    let job = jobs().get(&jid).cloned();
    let Some(job) = job else {
        return Err(Error::tool(format!(
            "No job {}. Call job_status without id to list all jobs.",
            py::py_str(id)
        )));
    };
    let seconds = py::float(&a.or("seconds", json!(0)))?;
    job.wait(seconds.min(state::run_wait()).max(0.0));
    Ok(job_text(jid, &job, Some(MAX_OUTPUT - 2000)).into())
}

pub fn t_stop_job(a: &Args) -> Res<ToolOut> {
    let id = a.req("id");
    let jid = py::int(id)?;
    let job = jobs().get(&jid).cloned();
    let Some(job) = job else {
        return Err(Error::tool(format!("No job {}.", py::py_str(id))));
    };
    if kill_if_running(&job, Killed::Stopped, false) {
        job.wait(10.0);
    }
    Ok(job_text(jid, &job, Some(3000)).into())
}

pub fn t_start_process(a: &Args) -> Res<ToolOut> {
    let dir = workdir(a)?;
    let ps = command_text(a)?;
    let log = temp_dir().join(format!("geminiforge_{}_{}.log", state::now() as i64, ffi::token_hex(3)));
    let pid = {
        let f = fs::File::create(&log).map_err(|e| py::os_error(&e, Some(&show(&log))))?;
        let f2 = f.try_clone()?;
        let mut cmd = shell(&ps, &dir);
        cmd.stdout(f).stderr(f2);
        let child = cmd.spawn().map_err(|e| py::os_error(&e, None))?;
        let pid = child.id();
        reap(child);
        pid
    };
    Ok(format!(
        "Started in the background (PID {pid}). Output is logged to {}; view it with read_file.",
        show(&log)
    )
    .into())
}

/// Let a background process go. Windows needs nothing; elsewhere a thread
/// waits for it so it does not stay behind as a zombie.
#[cfg(windows)]
fn reap(_child: Child) {}

#[cfg(not(windows))]
fn reap(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// `open`: the file, folder or web address in its default program (macOS
/// `open`, Linux `xdg-open`). A name that is neither, such as "Safari" or
/// "gedit", starts that application or program.
#[cfg(not(windows))]
pub fn t_open(a: &Args) -> Res<ToolOut> {
    use std::io::Read;
    let target = a.req("target");
    let text = py::py_str(target);
    let is_url = text.contains("://") || text.starts_with("mailto:");
    let path = if is_url { PathBuf::new() } else { resolve(target) };
    let mut cmd = if is_url || path.exists() {
        let mut c = Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
        c.arg(if is_url { text.clone() } else { show(&path) });
        c
    } else if cfg!(target_os = "macos") && !text.contains('/') {
        let mut c = Command::new("open");
        c.args(["-a", &text]);
        c
    } else if let Some(program) = (!text.contains('/')).then(|| find_program(&text)).flatten() {
        let mut c = Command::new(program);
        c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        reap(c.current_dir(state::cwd()).spawn().map_err(|e| py::os_error(&e, None))?);
        state::sleep(if state::fast_wait() { 0.5 } else { 1.5 });
        return Ok(format!("Opened: {text}").into());
    } else {
        return Err(Error::exc(
            "FileNotFoundError",
            format!("[Errno 2] No such file or directory: {}", py::repr_str(&show(&path))),
        ));
    };
    let mut child = cmd
        .current_dir(state::cwd())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::tool(format!(
                "{} is not installed, so nothing can be opened. On Linux install it with: sudo apt install xdg-utils",
                cmd.get_program().to_string_lossy()
            )),
            _ => py::os_error(&e, None),
        })?;
    // open and xdg-open return once the program has started; some xdg-open
    // setups keep running with it, which is fine too
    if state::poll_until(5.0, 0.05, || matches!(child.try_wait(), Ok(Some(_)))) {
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        if !ok {
            let mut err = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                let _ = pipe.read_to_string(&mut err);
            }
            return Err(Error::tool(format!("Could not open {text}: {}", py::strip(&err))));
        }
    } else {
        std::thread::spawn(move || {
            if let Some(mut pipe) = child.stderr.take() {
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            }
            let _ = child.wait();
        });
    }
    state::sleep(if state::fast_wait() { 0.5 } else { 1.5 });
    Ok(format!("Opened: {text}").into())
}

#[cfg(windows)]
pub fn t_open(a: &Args) -> Res<ToolOut> {
    let target = a.req("target");
    let path = expandvars(&py::py_str(target));
    let wide = ffi::wide(&path);
    let fast = state::fast_wait();
    let before = if fast { (crate::win::foreground(), crate::win::top_windows()) } else { (0, Vec::new()) };
    // os.startfile: ShellExecuteW with the default verb, SW_SHOWNORMAL
    let rc = unsafe {
        ffi::ShellExecuteW(0, std::ptr::null(), wide.as_ptr(), std::ptr::null(), std::ptr::null(), 1)
    };
    if rc <= 32 {
        let code = unsafe { ffi::GetLastError() };
        return Err(py::win_error(code, Some(&path)));
    }
    if fast {
        // until a new window shows up or another window comes to the front
        let (fg, wins) = before;
        state::poll_until(1.4, 0.05, || {
            crate::win::foreground() != fg
                || crate::win::top_windows().iter().any(|w| !wins.iter().any(|old| old.hwnd == w.hwnd))
        });
        state::sleep(0.1);
    } else {
        state::sleep(1.5);
    }
    Ok(format!("Opened: {}", py::py_str(target)).into())
}

pub fn t_wait(a: &Args) -> Res<ToolOut> {
    let seconds = a.or("seconds", json!(1));
    state::sleep(py::float(&seconds)?.min(30.0).max(0.0));
    Ok(format!("Waited {} seconds", py::py_str(&seconds)).into())
}

/// Test helper: stop and forget every job, deleting its log files.
pub fn clear_jobs() {
    let all: Vec<Arc<Job>> = {
        let mut map = jobs();
        let v = map.values().cloned().collect();
        map.clear();
        v
    };
    for job in all {
        if job.poll().is_none() {
            kill_tree(job.pid);
            job.wait(5.0);
        }
        let _ = fs::remove_file(&job.out);
        let _ = fs::remove_file(&job.err);
    }
}
