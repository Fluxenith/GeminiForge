//! OCR tools: ocr_window and click_text.

use crate::gui_tools::{click_without_focus, fit_width, new_capture_id, valid_button, validate_capture_id, with_restore_focus};
use crate::image;
use crate::input;
use crate::ocr;
use crate::py::{self, Error, Res};
use crate::state::{self, OcrEntry, OcrItem, OcrMeta, CAPTURE_TTL};
use crate::tools::{Args, ToolOut};
use crate::win;
use serde_json::{json, Value};

/// Resolve a target window to (hwnd, title, pid), restoring only if minimized/hidden.
fn resolve_win_hwnd(window: Option<&Value>, window_id: Option<&Value>) -> Res<(isize, String, u32)> {
    let window = window.filter(|v| !v.is_null());
    let window_id = window_id.filter(|v| !v.is_null());
    let (hwnd, title) = if window.is_some() || window_id.is_some() {
        let w = win::find_window(window, window_id)?;
        if w.min || w.hidden {
            win::activate(w.hwnd, w.hidden)?;
        }
        (w.hwnd, w.title)
    } else {
        let hwnd = win::foreground();
        if hwnd == 0 {
            return Err(Error::tool("No active window found"));
        }
        (hwnd, py::strip(&win::window_text(hwnd)).to_string())
    };
    Ok((hwnd, title, win::window_process_id(hwnd)))
}

/// Capture a window cleanly and run native Windows OCR with Set-of-Mark numbered boxes.
fn ocr_window(
    window: Option<&Value>, window_id: Option<&Value>, filter: &Value, overlay: bool, max: &Value,
) -> Res<ToolOut> {
    state::with(|s| {
        s.ocr.clear();
        s.ocr_meta = None;
    });
    let (hwnd, title, pid) = resolve_win_hwnd(window, window_id)?;
    let captured = win::capture_window_image(hwnd).and_then(|(img, vis, method)| {
        let (lang, items) = ocr::ocr_image(&img, 15.0)?;
        Ok((img, vis, method, lang, items))
    });
    let (img, (ox, oy, _, _), method, lang, mut items) =
        captured.map_err(|exc| Error::tool(format!("OCR failed on window_id={hwnd}: {}", exc.py_str())))?;

    if py::truthy(filter) {
        let f = py::strip(&py::py_str(filter)).to_lowercase();
        if !f.is_empty() {
            items.retain(|it| it.text.to_lowercase().contains(&f));
        }
    }
    let cap = py::int(max)?.clamp(1, 300) as usize;
    let truncated = items.len() > cap;
    items.truncate(cap);

    let (disp, scale) = fit_width(img);
    let cid = new_capture_id();
    let now = state::now();
    state::with(|s| {
        s.shot_scale = scale;
        s.shot_origin = (ox, oy);
        s.shot_size = Some((disp.w, disp.h));
        s.shot_hwnd = Some(hwnd);
        s.shot_capture_id = Some(cid.clone());
        s.shot_time = now;
    });

    let mut lines: Vec<String> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let (sx, sy) = (py::round(it.cx as f64 / scale), py::round(it.cy as f64 / scale));
        let ellipsis = if py::len(&it.text) > 100 { "…" } else { "" };
        lines.push(format!("[{}] \"{}{ellipsis}\" at ({sx},{sy})", i + 1, py::prefix(&it.text, 100)));
    }
    state::with(|s| {
        s.ocr = items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let entry = OcrEntry {
                    item: it.clone(),
                    hwnd,
                    title: title.clone(),
                    pid,
                    rx: ox as i64 + it.cx,
                    ry: oy as i64 + it.cy,
                    origin: (ox, oy),
                    capture_id: cid.clone(),
                    captured_at: now,
                };
                (i as i64 + 1, entry)
            })
            .collect();
        s.ocr_meta = Some(OcrMeta {
            hwnd,
            title: title.clone(),
            pid,
            capture_id: cid.clone(),
            captured_at: now,
            lang: lang.clone(),
            method: method.to_string(),
        });
    });
    let head = format!(
        "OCR window \"{}\" window_id={hwnd} capture_id={cid} lang={lang} method={method} ({} text block(s); click with click_text {{id, window_id, capture_id}} or click_text {{text, window_id}}):",
        py::prefix(&title, 60),
        items.len()
    );
    let out_text = if lines.is_empty() {
        format!("{head}\n(no OCR text detected)")
    } else {
        if truncated {
            lines.push(format!("... (capped at {cap}; narrow with filter)"));
        }
        format!("{head}\n{}", lines.join("\n"))
    };
    if overlay {
        let indexed: Vec<(i64, &OcrItem)> = items.iter().enumerate().map(|(i, it)| (i as i64 + 1, it)).collect();
        let marked = ocr::draw_som_overlay(&disp, &indexed, scale);
        return Ok(ToolOut { text: out_text, image: Some(image::jpeg_base64(&marked, 84)?) });
    }
    Ok(out_text.into())
}

pub fn t_ocr_window(a: &Args) -> Res<ToolOut> {
    ocr_window(
        a.get("window"),
        a.get("window_id"),
        &a.or("filter", Value::Null),
        a.truthy("overlay", true),
        &a.or("max", json!(120)),
    )
}

/// Click OCR-detected text by mark id or substring, with optional background, cursor-jump or flash-restore click.
pub fn t_click_text(a: &Args) -> Res<ToolOut> {
    let button = valid_button(&a.or("button", json!("left")))?;
    let (text, id) = (a.opt("text"), a.opt("id"));
    let (window_id, capture_id) = (a.opt("window_id"), a.opt("capture_id"));
    let clicks = a.or("clicks", json!(1));
    if id.is_none() && text.map(|t| py::strip(&py::py_str(t)).is_empty()).unwrap_or(true) {
        return Err(Error::tool("Provide id (from ocr_window) or text."));
    }
    let wanted_hwnd = window_id.map(|v| py::int(v).map(|i| i as isize));

    let (hwnd, rx, ry, label) = if let Some(id) = id {
        let key = py::int(id)?;
        let Some(entry) = state::with(|s| s.ocr.get(&key).cloned()) else {
            return Err(Error::tool(format!("No OCR mark with id {}; run ocr_window first.", py::py_str(id))));
        };
        if let Some(wanted) = wanted_hwnd {
            if entry.hwnd != wanted? {
                return Err(Error::tool("That OCR mark is not in the given window_id; run ocr_window again."));
            }
        }
        if capture_id.is_some() {
            validate_capture_id(capture_id, Some(&entry.capture_id), Some(entry.captured_at))?;
        } else if state::now() - entry.captured_at > CAPTURE_TTL {
            return Err(Error::tool(format!(
                "capture_expired: OCR snapshot is older than {}s; run ocr_window again.",
                CAPTURE_TTL as i64
            )));
        }
        win::validate_ui_window(entry.hwnd, &entry.title, entry.pid)?;
        let label = format!("[{}] \"{}\"", py::py_str(id), py::prefix(&entry.item.text, 40));
        (entry.hwnd, entry.rx as i32, entry.ry as i32, label)
    } else {
        let q = py::strip(&py::py_str(text.unwrap_or(&Value::Null))).to_string();
        let wanted = wanted_hwnd.transpose()?;
        let mut meta = state::with(|s| s.ocr_meta.clone());
        let cache_ok = state::with(|s| !s.ocr.is_empty())
            && meta
                .as_ref()
                .map(|m| state::now() - m.captured_at <= CAPTURE_TTL && wanted.map(|h| m.hwnd == h).unwrap_or(true))
                .unwrap_or(false);
        if capture_id.is_some() {
            validate_capture_id(
                capture_id,
                meta.as_ref().map(|m| m.capture_id.as_str()),
                Some(meta.as_ref().map(|m| m.captured_at).unwrap_or(0.0)),
            )?;
            if !cache_ok {
                return Err(Error::tool("capture_expired: no matching OCR snapshot for that window_id; run ocr_window again."));
            }
        } else if !cache_ok {
            ocr_window(None, window_id, &Value::Null, false, &json!(120))?;
            meta = state::with(|s| s.ocr_meta.clone());
        }
        let entries: Vec<(i64, OcrEntry)> = state::with(|s| s.ocr.iter().map(|(k, v)| (*k, v.clone())).collect());
        let Some(meta) = meta.filter(|_| !entries.is_empty()) else {
            return Err(Error::tool(format!("No OCR text matching \"{q}\" found.")));
        };
        win::validate_ui_window(meta.hwnd, &meta.title, meta.pid)?;
        let q_low = q.to_lowercase();
        let mut hits: Vec<&(i64, OcrEntry)> = entries.iter().filter(|(_, v)| v.item.text.to_lowercase() == q_low).collect();
        if hits.is_empty() {
            hits = entries.iter().filter(|(_, v)| v.item.text.to_lowercase().contains(&q_low)).collect();
        }
        if hits.is_empty() {
            return Err(Error::tool(format!("No OCR text matching \"{q}\" found in window_id={}.", meta.hwnd)));
        }
        if hits.len() > 1 {
            let preview: Vec<String> =
                hits.iter().take(8).map(|(i, v)| format!("[{i}]\"{}\"", py::prefix(&v.item.text, 24))).collect();
            return Err(Error::tool(format!(
                "\"{q}\" matches multiple OCR blocks: {}. Use a fuller text or mark id.",
                preview.join(", ")
            )));
        }
        let (hit_id, entry) = hits[0];
        let (bx, by, bw, bh) = ocr::find_substring_box(&entry.item, &q);
        let (ox, oy) = entry.origin;
        let (rx, ry) = (ox as i64 + bx + bw.div_euclid(2), oy as i64 + by + bh.div_euclid(2));
        (entry.hwnd, rx as i32, ry as i32, format!("[{hit_id}] \"{}\"", py::prefix(&q, 40)))
    };

    state::with(|s| s.focus_point = Some((rx, ry)));
    let (background, cursor_jump) = (a.truthy("background", false), a.truthy("cursor_jump", false));
    if background || cursor_jump {
        return Ok(click_without_focus(hwnd, rx, ry, &button, &clicks, background, &format!("OCR {label}"))?.into());
    }
    let restore_focus = a.truthy("restore_focus", false);
    with_restore_focus(restore_focus, Some(hwnd), || {
        win::activate(hwnd, false)?;
        input::click(rx, ry, py::int(&clicks)?, 0.08, &button)
    })?;
    let suffix = if restore_focus { " (restored previous window)" } else { "" };
    Ok(format!("Clicked OCR {label} at ({rx},{ry}) in window_id={hwnd}{suffix}").into())
}
