//! GeminiForge local tool server (Rust).
//!
//!     geminiforge-server                      the tool server for the Chrome extension (127.0.0.1:18765)
//!     geminiforge-server mcp                  the MCP entry for Gemini's "@ custom app" (127.0.0.1:18767)
//!     geminiforge-server backups [list [N] | restore ID]
//!     geminiforge-server approve TOOL ARGS SECONDS   the confirmation popup the MCP entry shows
//!
//! Runs on 127.0.0.1 only. The Chrome extension sends tool calls that Gemini
//! wrote in gemini.google.com, this server runs them on this computer and
//! returns the result (text, and a screenshot for computer-use tools).

#[cfg(windows)]
use geminiforge::ffi;
use geminiforge::{approve, backups, http, lang, mcp, state};
use std::io::{IsTerminal, Write};

/// Must run before any mouse or screen call so coordinates match real
/// screen pixels on displays with Windows scaling (125%, 150%...).
#[cfg(windows)]
fn dpi_aware() {
    unsafe {
        if ffi::SetProcessDpiAwareness(2) != 0 {
            ffi::SetProcessDPIAware();
        }
    }
}

/// The desktop tools that need it are Windows-only so far.
#[cfg(not(windows))]
fn dpi_aware() {}

#[cfg(windows)]
fn init_input() {
    geminiforge::input::init();
}

#[cfg(not(windows))]
fn init_input() {}

fn tool_server() {
    dpi_aware();
    let port: u16 = std::env::var("GF_PORT").ok().and_then(|p| p.trim().parse().ok()).unwrap_or(18765);
    let listener = match http::bind(port) {
        Ok(l) => l,
        Err(_) => {
            println!("{}", lang::t("srvPortInUse", &[("port", &port.to_string())]));
            if std::io::stdin().is_terminal() {
                print!("{}", lang::t("pressEnterToExit", &[]));
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
            }
            return;
        }
    };
    init_input();
    println!("{}", lang::t("srvStarted", &[("url", &format!("http://{}:{}", state::HOST, port))]));
    println!("{}", lang::t("srvCwd", &[("path", &state::cwd())]));
    // the mouse-corner emergency stop only exists where the mouse tools do
    println!("{}", lang::t(if cfg!(windows) { "srvStop" } else { "srvStopPlain" }, &[]));
    if state::fast_wait() {
        println!("Fast waits: on (GF_FAST_WAIT=1). Set GF_FAST_WAIT=0 for fixed waits.");
    }
    let _ = std::io::stdout().flush();
    http::serve(listener);
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match argv.first().map(|s| s.as_str()) {
        None => tool_server(),
        Some("mcp") => {
            dpi_aware();
            init_input();
            mcp::run();
        }
        Some("backups") => std::process::exit(backups::cli(&argv[1..])),
        // the popup is left to Windows to scale, so it is the same size on every display
        Some("approve") => std::process::exit(approve::main(&argv[1..])),
        Some(other) => {
            eprintln!("Unknown command: {other}\nUse no argument for the tool server, or: mcp | backups [list [N] | restore ID]");
            std::process::exit(2);
        }
    }
}
