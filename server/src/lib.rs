//! GeminiForge local tool server.
//!
//! Windows has every tool. On macOS and Linux the file, command and MCP parts
//! run the same code; the platform modules come from `unix/`, screenshots use
//! the system's capture program, and the other desktop tools (mouse and
//! keyboard, windows, UI elements, OCR) are not there yet.

pub mod approve;
pub mod backups;
pub mod crypto;
pub mod fs_tools;
pub mod http;
pub mod image;
pub mod lang;
pub mod mcp;
pub mod proc_tools;
pub mod py;
pub mod state;
pub mod tools;

#[cfg(windows)]
pub mod ffi;
#[cfg(windows)]
pub mod gui_tools;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod ocr;
#[cfg(windows)]
pub mod ocr_tools;
#[cfg(windows)]
pub mod ui_tools;
#[cfg(windows)]
pub mod uia;
#[cfg(windows)]
pub mod win;

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use unix::{ffi, gui_tools, ocr_tools, ui_tools};
