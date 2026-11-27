//! macOS and Linux versions of the platform modules.
//!
//! `ffi` and `popup` stand in for their Windows counterparts. The desktop
//! tools (screen, mouse and keyboard, windows, UI elements, OCR) are not
//! there yet: `tools::call_tool` turns them away before they run, and the
//! functions below only keep the tool table the same on every system.

/// Tool functions for the desktop tools this system does not have yet.
macro_rules! not_here {
    ($($name:ident),* $(,)?) => {
        $(
            pub fn $name(_a: &crate::tools::Args) -> crate::py::Res<crate::tools::ToolOut> {
                Err(crate::tools::unavailable(stringify!($name).trim_start_matches("t_")))
            }
        )*
    };
}

pub mod ffi;
pub mod gui_tools;
pub mod popup;

pub mod ui_tools {
    not_here!(
        t_scroll, t_ui_elements, t_read_window, t_click_element, t_click_elements, t_type_element, t_wait_for,
        t_gui_batch,
    );
}

pub mod ocr_tools {
    not_here!(t_ocr_window, t_click_text);
}
