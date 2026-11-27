//! UI Automation tools: ui_elements, read_window, click_element,
//! click_elements, type_element, scroll, wait_for, gui_batch.
//! They let Gemini click buttons by name instead of guessing pixel positions.

use crate::gui_tools::{opt_hwnd, to_real, to_shot, type_text, with_restore_focus};
use crate::input;
use crate::ocr;
use crate::py::{self, Error, Res};
use crate::state::{self, fast_wait, now, poll_until, sleep, UiInfo};
use crate::tools::{self, Args, ToolOut};
use crate::uia::{self, Ctl, Ctx, ScrollPat};
use crate::win;
use serde_json::{json, Map, Value};
use std::rc::Rc;
use windows::Win32::UI::Accessibility::{
    ScrollAmount_LargeDecrement, ScrollAmount_LargeIncrement, ScrollAmount_SmallDecrement,
    ScrollAmount_SmallIncrement,
};

const CLICKABLE: &[&str] = &[
    "ButtonControl", "MenuItemControl", "ListItemControl", "TreeItemControl", "TabItemControl",
    "HyperlinkControl", "EditControl", "CheckBoxControl", "RadioButtonControl", "ComboBoxControl",
    "SplitButtonControl", "DataItemControl", "DocumentControl", "SpinnerControl",
];
const VALUE_CONTROLS: &[&str] = &["EditControl", "ComboBoxControl", "DocumentControl", "SpinnerControl"];
const SCROLL_CONTAINERS: &[&str] = &[
    "DocumentControl", "PaneControl", "GroupControl", "ListControl", "TreeControl", "TableControl",
    "DataGridControl", "EditControl", "CustomControl", "WindowControl",
];
const UI_WALK_DEPTH: u32 = 64;
const UI_WALK_LIMIT: usize = 12000;

type Tree = Vec<(Ctl, u32)>;

/// A COM failure outside any try block of the Python code surfaces as a COMError.
fn w<T>(r: windows::core::Result<T>) -> Res<T> {
    r.map_err(|e| uia::com_error(&e))
}

fn is_icon_glyphs(text: &str) -> bool {
    text.chars().all(|c| ('\u{e000}'..='\u{f8ff}').contains(&c) || py::is_space(c))
}

/// Return (value_text, is_password) without ever reading password fields.
fn control_value_info(c: &Ctl, max_len: i32, use_text_pattern: bool) -> (String, bool) {
    if c.is_password().unwrap_or(false) {
        return (String::new(), true);
    }
    let mut val = c.value_pattern_value().ok().flatten().unwrap_or_default();
    let ct = c.type_name().unwrap_or("");
    if val.is_empty() && ct == "ComboBoxControl" {
        val = c.legacy_value().ok().flatten().unwrap_or_default();
    }
    if val.is_empty() && use_text_pattern && (ct == "EditControl" || ct == "DocumentControl") {
        val = c.document_text(max_len + 1).ok().flatten().unwrap_or_default();
    }
    (val, false)
}

fn toggle_state(c: &Ctl) -> &'static str {
    match c.toggle_state() {
        Ok(Some(1)) => "checked",
        Ok(Some(0)) => "unchecked",
        Ok(Some(2)) => "indeterminate",
        _ => "",
    }
}

fn is_selected(c: &Ctl) -> bool {
    matches!(c.is_selected(), Ok(Some(true)))
}

fn scroll_into_view(c: &Ctl) -> bool {
    if !matches!(c.is_offscreen(), Ok(true)) {
        return false;
    }
    match c.scroll_into_view() {
        Ok(true) => {
            if fast_wait() {
                // until the element reports that it is on screen (same upper limit as the fixed waits)
                poll_until(0.72, 0.03, || matches!(c.is_offscreen(), Ok(false)));
            } else {
                sleep(0.25);
            }
            true
        }
        _ => false,
    }
}

/// A fingerprint of everything the tools read from a cached tree: a change
/// means the window reacted (a text, a check mark, a selection, a new dialog).
fn tree_signature(tree: &Tree) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tree.len().hash(&mut h);
    for (c, depth) in tree {
        depth.hash(&mut h);
        c.type_id().ok().hash(&mut h);
        c.name().ok().hash(&mut h);
        c.is_offscreen().ok().hash(&mut h);
        c.is_enabled().ok().hash(&mut h);
        c.toggle_state().ok().flatten().hash(&mut h);
        c.is_selected().ok().flatten().hash(&mut h);
        if let Ok(r) = c.rect() {
            (r.left, r.top, r.right, r.bottom).hash(&mut h);
        }
    }
    h.finish()
}

/// Fast wait after a click: read the window again until it differs from
/// `before` and two reads in a row agree, or `max` seconds have passed.
/// Returns the last read (None when the window is gone or cannot be cached).
fn settle_tree(ctx: &Rc<Ctx>, hwnd: isize, before: &Tree, max: f64) -> Option<Tree> {
    let before_sig = tree_signature(before);
    let deadline = now() + max;
    let mut last_sig: Option<u64> = None;
    loop {
        match window_tree(ctx, hwnd) {
            Some(cur) => {
                let sig = tree_signature(&cur);
                if (sig != before_sig && last_sig == Some(sig)) || now() >= deadline {
                    return Some(cur);
                }
                last_sig = Some(sig);
            }
            None => {
                // A window busy with the click cannot be read for a moment: try
                // again rather than give up (giving up means a slow walk later).
                if !win::is_window(hwnd) || now() >= deadline {
                    return None;
                }
                last_sig = None;
            }
        }
        sleep(0.04);
    }
}

/// Fast wait after a scroll call: until the position has moved (or is where
/// it was asked to be) and two reads in a row agree, or `max` seconds have passed.
fn settle_percent(sp: &ScrollPat, before: Option<f64>, target: Option<f64>, max: f64) {
    let mut last: Option<f64> = None;
    let started = now();
    poll_until(max, 0.03, || {
        let cur = sp.percent().0;
        let moved = match (cur, before) {
            (Some(c), Some(b)) => (c - b).abs() >= 0.1,
            _ => false,
        };
        let arrived = match (cur, target) {
            (Some(c), Some(t)) => (c - t).abs() < 0.5,
            _ => false,
        };
        let steady = cur.is_some() && cur == last;
        last = cur;
        // nothing moved at all after a quarter of a second: the container was already there
        ((moved || arrived) && steady) || (!moved && steady && now() - started >= 0.25)
    });
}

fn control_belongs_to_hwnd(ctx: &Ctx, c: &Ctl, hwnd: Option<isize>) -> bool {
    let Some(hwnd) = hwnd.filter(|h| *h != 0) else { return true };
    let mut cur = Some(c.clone());
    for _ in 0..40 {
        let Some(c) = cur else { return false };
        if c.native_hwnd() == hwnd {
            return true;
        }
        cur = match ctx.parent(&c) {
            Ok(p) => p,
            Err(_) => return false,
        };
    }
    false
}

fn find_scrollable_in_window(
    ctx: &Rc<Ctx>, hwnd: Option<isize>, point: Option<(i32, i32)>,
) -> Res<(Option<Ctl>, Option<ScrollPat>)> {
    let root = match hwnd.filter(|h| *h != 0) {
        Some(h) => w(ctx.control_from_handle(h))?,
        None => None,
    };
    let Some(root) = root else { return Ok((None, None)) };
    let mut containing: Vec<(i64, Ctl, ScrollPat)> = Vec::new();
    let mut candidates: Vec<(i64, Ctl, ScrollPat)> = Vec::new();
    let (mut n, deadline) = (0, now() + 4.0);
    for (c, _depth) in ctx.walk(&root, true, 40) {
        n += 1;
        if n > 4000 || now() > deadline {
            break;
        }
        let mut check = || -> Option<()> {
            let ct = c.type_name().ok()?;
            if !SCROLL_CONTAINERS.contains(&ct) || c.is_offscreen().ok()? {
                return None;
            }
            let r = c.rect().ok()?;
            let (wd, ht) = (r.width(), r.height());
            if wd < 80 || ht < 80 {
                return None;
            }
            let sp = c.scroll_pattern().ok()??;
            if !sp.vertically_scrollable() {
                return None;
            }
            let area = wd as i64 * ht as i64;
            if let Some((px, py_)) = point {
                if r.left <= px && px < r.right && r.top <= py_ && py_ < r.bottom {
                    containing.push((area, c.clone(), sp.clone()));
                }
            }
            candidates.push((area, c.clone(), sp));
            Some(())
        };
        check();
    }
    // the smallest container under the point, else the largest one (first of equals)
    let mut pick: Option<(i64, Ctl, ScrollPat)> = None;
    if !containing.is_empty() {
        for item in containing {
            if pick.as_ref().map(|p| item.0 < p.0).unwrap_or(true) {
                pick = Some(item);
            }
        }
    } else {
        for item in candidates {
            if pick.as_ref().map(|p| item.0 > p.0).unwrap_or(true) {
                pick = Some(item);
            }
        }
    }
    Ok(match pick {
        Some((_, c, sp)) => (Some(c.live()), Some(sp)),
        None => (None, None),
    })
}

fn find_scrollable_at(ctx: &Rc<Ctx>, rx: i32, ry: i32, hwnd: Option<isize>) -> Res<(Option<Ctl>, Option<ScrollPat>)> {
    let mut cur = ctx.control_from_point(rx, ry).ok().flatten();
    for _ in 0..40 {
        let Some(c) = cur else { break };
        if let Some(sp) = c.scroll_pattern().ok().flatten() {
            if sp.vertically_scrollable() && control_belongs_to_hwnd(ctx, &c, hwnd) {
                return Ok((Some(c), Some(sp)));
            }
        }
        if hwnd.map(|h| h != 0 && c.native_hwnd() == h).unwrap_or(false) {
            break;
        }
        cur = match ctx.parent(&c) {
            Ok(p) => p,
            Err(_) => break,
        };
    }
    if hwnd.map(|h| h != 0).unwrap_or(false) {
        return find_scrollable_in_window(ctx, hwnd, Some((rx, ry)));
    }
    Ok((None, None))
}

/// The page document inside a browser-like window, else the window itself.
/// Returns (root, url, the chosen root's cached descendants if it could be cached).
fn find_content_root(ctx: &Rc<Ctx>, target: &Ctl) -> (Ctl, Option<String>, Tree) {
    let cached_nodes = ctx.snapshot(target, false, UI_WALK_DEPTH);
    let cached = cached_nodes.is_some();
    let mut best: Option<(Ctl, (i64, bool, i64), usize, u32)> = None;
    let (mut n, deadline) = (0, now() + 2.0);
    let mut look = |i: usize, c: &Ctl, depth: u32| -> bool {
        if depth > 14 {
            return true;
        }
        n += 1;
        if !cached && (n > 500 || now() > deadline) {
            return false; // the slow walk needs a budget
        }
        let key = (|| -> Option<(i64, bool, i64)> {
            if c.type_name().ok()? != "DocumentControl" || c.is_offscreen().ok()? {
                return None;
            }
            let r = c.rect().ok()?;
            let (wd, ht) = (r.width(), r.height());
            if wd < 200 || ht < 150 {
                return None;
            }
            // Same size: the page document carries the page title, an empty
            // wrapper around it does not.
            Some((wd as i64 * ht as i64, !c.name().ok()?.is_empty(), -(depth as i64)))
        })();
        if let Some(key) = key {
            if best.as_ref().map(|b| key > b.1).unwrap_or(true) {
                best = Some((c.clone(), key, i, depth));
            }
        }
        true
    };
    let mut tree = Tree::new();
    match &cached_nodes {
        Some(nodes) => {
            for (i, (c, depth)) in nodes.iter().enumerate() {
                look(i, c, *depth);
            }
        }
        None => {
            for (i, (c, depth)) in ctx.walk_live(target, false, 14).enumerate() {
                if !look(i, &c, depth) {
                    break;
                }
            }
        }
    }
    if let Some(nodes) = cached_nodes {
        match &best {
            None => tree = nodes,
            Some((_, _, at, at_depth)) => {
                for item in nodes.into_iter().skip(at + 1) {
                    if item.1 <= *at_depth {
                        break;
                    }
                    tree.push(item);
                }
            }
        }
    }
    match best {
        Some((c, ..)) => {
            let live = c.live();
            let (url, _) = control_value_info(&live, 300, false);
            (live, (!url.is_empty()).then_some(url), tree)
        }
        None => (target.clone(), None, tree),
    }
}

/// One cached read of a window's controls for helpers that look at the same
/// moment, or None when it cannot be cached (each helper then walks by itself).
fn window_tree(ctx: &Rc<Ctx>, hwnd: isize) -> Option<Tree> {
    let root = ctx.control_from_handle(hwnd).ok().flatten()?;
    ctx.snapshot(&root, false, UI_WALK_DEPTH)
}

/// Short visible texts in a window, so Gemini can check a result without a screenshot.
fn window_texts(ctx: &Rc<Ctx>, hwnd: isize, limit: usize, tree: Option<&Tree>) -> Res<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let Some(root) = w(ctx.control_from_handle(hwnd))? else { return Ok(out) };
    let mut n = 0;
    let mut visit = |c: &Ctl, depth: u32, out: &mut Vec<String>| -> bool {
        if depth > 40 {
            return true;
        }
        n += 1;
        if n > 4000 || out.len() >= limit {
            return false;
        }
        let text = (|| -> Option<String> {
            if c.type_name().ok()? != "TextControl" || c.is_offscreen().ok()? {
                return None;
            }
            let t = py::clean_one_line(&c.name().ok()?);
            (!is_icon_glyphs(&t) && py::len(&t) <= 60).then_some(t)
        })();
        if let Some(t) = text {
            if !out.contains(&t) {
                out.push(t);
            }
        }
        true
    };
    match tree {
        Some(tree) => {
            for (c, depth) in tree {
                if !visit(c, *depth, &mut out) {
                    break;
                }
            }
        }
        None => {
            for (c, depth) in ctx.walk(&root, false, 40) {
                if !visit(&c, depth, &mut out) {
                    break;
                }
            }
        }
    }
    Ok(out)
}

fn resolve_ui_target(ctx: &Rc<Ctx>, window: Option<&Value>, window_id: Option<&Value>) -> Res<(Ctl, isize, String)> {
    let window_id = window_id.filter(|v| !v.is_null());
    let pinned = if window.map(py::truthy).unwrap_or(false) || window_id.is_some() {
        let wn = win::find_window(window, window_id)?;
        if wn.min || wn.hidden {
            win::activate(wn.hwnd, wn.hidden)?; // hidden/minimized windows expose no buttons until shown
        }
        Some(wn)
    } else {
        None
    };
    let target = match &pinned {
        Some(wn) => w(ctx.control_from_handle(wn.hwnd))?,
        None => w(ctx.control_from_handle(win::foreground()))?,
    };
    let Some(target) = target else { return Err(Error::tool("No active window found")) };
    let hwnd = target.native_hwnd();
    let name = w(target.name())?;
    let title = py::strip(&name).to_string();
    if let Some(wn) = &pinned {
        if hwnd != wn.hwnd || title != py::strip(&wn.title) {
            return Err(Error::tool(
                "The window changed while being read; run list_windows again. No other program was read instead.",
            ));
        }
    }
    Ok((target, hwnd, title))
}

struct Item {
    idx: i64,
    ct: &'static str,
    name: String,
    val: String,
    is_pw: bool,
    flags: Vec<&'static str>,
    offscreen: bool,
    x: i32,
    y: i32,
}

enum Scan {
    Skip,
    Keep(Item),
    More,
}

fn show_value(val: &str) -> String {
    val.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\\n")
}

pub fn t_ui_elements(a: &Args) -> Res<ToolOut> {
    state::with(|s| s.ui.clear()); // a failed observation must not leave earlier elements usable
    let ctx = uia::ctx()?;
    let (target, hwnd, title) = resolve_ui_target(&ctx, a.get("window"), a.get("window_id"))?;
    let process_id = win::window_process_id(hwnd);
    let filter = a.or("filter", Value::Null);
    let f = py::truthy(&filter).then(|| py::py_str(&filter).to_lowercase());
    let limit = py::int(&a.or("max", json!(150)))?.clamp(1, 500) as usize;
    let start_idx = py::int(&a.or("offset", json!(1)))?.max(1);
    let include_text = a.truthy("include_text", false);
    let include_offscreen = a.truthy("include_offscreen", false);

    let mut items: Vec<Item> = Vec::new();
    let (mut visited, mut matched) = (0usize, 0i64);
    let (mut has_more, mut stopped_early) = (false, None);
    let deadline = now() + 10.0;
    for (c, _depth) in ctx.walk(&target, false, UI_WALK_DEPTH) {
        visited += 1;
        if visited > UI_WALK_LIMIT {
            stopped_early = Some("node limit");
            break;
        }
        if now() > deadline {
            stopped_early = Some("time limit");
            break;
        }
        let mut scan = || -> Option<Scan> {
            let ct = c.type_name().ok()?;
            if !CLICKABLE.contains(&ct) && !(include_text && ct == "TextControl") {
                return Some(Scan::Skip);
            }
            let offscreen = c.is_offscreen().ok()?;
            if !include_offscreen && offscreen {
                return Some(Scan::Skip);
            }
            let name = py::clean_one_line(&c.name().ok()?);
            let (mut val, mut is_pw, mut val_loaded) = (String::new(), false, false);
            if name.is_empty() && ct != "EditControl" && ct != "ComboBoxControl" {
                return Some(Scan::Skip);
            }
            let is_value = VALUE_CONTROLS.contains(&ct);
            if let Some(f) = &f {
                if !name.to_lowercase().contains(f.as_str()) {
                    if !is_value {
                        return Some(Scan::Skip);
                    }
                    (val, is_pw) = control_value_info(&c, 300, ct == "EditControl");
                    val_loaded = true;
                    if is_pw || !val.to_lowercase().contains(f.as_str()) {
                        return Some(Scan::Skip);
                    }
                }
            }
            let r = c.rect().ok()?;
            if !offscreen && (r.width() <= 0 || r.height() <= 0) {
                return Some(Scan::Skip);
            }
            if !val_loaded && is_value {
                (val, is_pw) = control_value_info(&c, 300, ct == "EditControl");
            }
            if name.is_empty() && ct == "ComboBoxControl" && val.is_empty() && !is_pw {
                return Some(Scan::Skip);
            }
            let mut flags: Vec<&'static str> = Vec::new();
            if ct == "CheckBoxControl" {
                let st = toggle_state(&c);
                if !st.is_empty() {
                    flags.push(st);
                }
            } else if (ct == "RadioButtonControl" || ct == "TabItemControl") && is_selected(&c) {
                flags.push("selected");
            }
            if CLICKABLE.contains(&ct) && !c.is_enabled().ok()? {
                flags.push("disabled");
            }
            if offscreen {
                flags.push("offscreen");
            }
            matched += 1;
            if matched < start_idx {
                return Some(Scan::Skip);
            }
            if items.len() >= limit {
                return Some(Scan::More);
            }
            let (x, y) = if r.width() > 0 && r.height() > 0 { (r.xcenter(), r.ycenter()) } else { (0, 0) };
            Some(Scan::Keep(Item { idx: matched, ct, name, val, is_pw, flags, offscreen, x, y }))
        };
        match scan() {
            Some(Scan::Keep(item)) => items.push(item),
            Some(Scan::More) => {
                has_more = true;
                break;
            }
            _ => {}
        }
    }
    state::with(|s| {
        s.ui = items
            .iter()
            .map(|it| {
                let info = UiInfo {
                    hwnd,
                    title: title.clone(),
                    pid: process_id,
                    ctype: it.ct.to_string(),
                    name: it.name.clone(),
                    x: it.x,
                    y: it.y,
                };
                (it.idx, info)
            })
            .collect();
    });
    let shot_taken = state::with(|s| s.shot_size.is_some());
    let mut lines: Vec<String> = Vec::new();
    for it in &items {
        let ct_short = it.ct.replace("Control", "");
        let name_cap = if it.ct == "TextControl" { 300 } else { 90 };
        let disp_name = if py::len(&it.name) > name_cap {
            format!("{}…", py::prefix(&it.name, name_cap))
        } else {
            it.name.clone()
        };
        let mut parts = vec![format!("[{}] {} \"{}\"", it.idx, ct_short, disp_name)];
        if it.is_pw {
            parts.push("value=\"[password]\"".to_string());
        } else if !it.val.is_empty() {
            let v_disp = show_value(&it.val);
            if py::len(&v_disp) > 300 {
                parts.push(format!("value=\"{}…\" ({} chars)", py::prefix(&v_disp, 300), py::len(&v_disp)));
            } else {
                parts.push(format!("value=\"{v_disp}\""));
            }
        } else if it.ct == "EditControl" {
            parts.push("value=\"\"".to_string());
        }
        if !it.flags.is_empty() {
            parts.push(it.flags.iter().map(|f| format!("[{f}]")).collect::<Vec<_>>().join(" "));
        }
        if !it.offscreen {
            match to_shot(it.x, it.y) {
                Some((sx, sy)) => parts.push(format!("screenshot ≈{sx},{sy}")),
                None if shot_taken => parts.push("not on the last screenshot's monitor".to_string()),
                None => {}
            }
        }
        lines.push(parts.join(" "));
    }
    let head = format!(
        "Elements in window \"{}\" window_id={hwnd} ({} found; click with click_element {{id, window_id}}):",
        py::prefix(&title, 60),
        items.len()
    );
    if items.is_empty() {
        return Ok(format!(
            "{head}\n(none found; try another filter, include_text=true, include_offscreen=true, or read_window)"
        )
        .into());
    }
    if has_more {
        lines.push(format!(
            "... (showing {}-{}; more elements exist — continue with offset={} or narrow with filter)",
            start_idx,
            start_idx + items.len() as i64 - 1,
            start_idx + items.len() as i64
        ));
    } else if let Some(why) = stopped_early {
        lines.push(format!(
            "... (scan stopped at {why} after {visited} controls; narrow with filter or use read_window)"
        ));
    }
    Ok(format!("{head}\n{}", lines.join("\n")).into())
}

/// Read readable text, field values and form states from a window (scoped to active web page if any).
pub fn t_read_window(a: &Args) -> Res<ToolOut> {
    let ctx = uia::ctx()?;
    let (target, hwnd, title) = resolve_ui_target(&ctx, a.get("window"), a.get("window_id"))?;
    let include_offscreen = a.truthy("include_offscreen", true);
    let (root, page_url, tree) = find_content_root(&ctx, &target);
    let mut lines: Vec<String> = Vec::new();
    let mut last_text: Option<String> = None;
    let (mut visited, mut stopped_early) = (0usize, None);
    let deadline = now() + 10.0;
    let mut visit = |c: &Ctl, lines: &mut Vec<String>| -> Option<()> {
        if !include_offscreen && c.is_offscreen().ok()? {
            return None;
        }
        let ct = c.type_name().ok()?;
        macro_rules! push {
            ($line:expr, $remember:expr) => {{
                lines.push($line);
                last_text = Some($remember);
            }};
        }
        match ct {
            "TextControl" => {
                let name = c.name().ok()?;
                let txt = py::strip(&name).to_string();
                if !txt.is_empty() && !is_icon_glyphs(&txt) && last_text.as_deref() != Some(txt.as_str()) {
                    push!(txt.clone(), txt);
                }
            }
            "EditControl" | "ComboBoxControl" | "SpinnerControl" => {
                let name = py::clean_one_line(&c.name().ok()?);
                let (val, is_pw) = control_value_info(c, 2000, ct == "EditControl");
                let ct_short = ct.replace("Control", "");
                let v_str = if is_pw { "[password]".to_string() } else { val.replace("\r\n", "\n").replace('\r', "\n") };
                let line = if name.is_empty() {
                    format!("[{ct_short}: \"{v_str}\"]")
                } else {
                    format!("[{ct_short} \"{name}\": \"{v_str}\"]")
                };
                push!(line.clone(), line);
            }
            "CheckBoxControl" => {
                let name = py::clean_one_line(&c.name().ok()?);
                let st = match toggle_state(c) {
                    "" => "unknown",
                    s => s,
                };
                let line =
                    if name.is_empty() { format!("[CheckBox: {st}]") } else { format!("[CheckBox \"{name}\": {st}]") };
                push!(line.clone(), line);
            }
            "RadioButtonControl" => {
                let name = py::clean_one_line(&c.name().ok()?);
                let st = if is_selected(c) { "selected" } else { "unselected" };
                let line = if name.is_empty() { format!("[Radio: {st}]") } else { format!("[Radio \"{name}\": {st}]") };
                push!(line.clone(), line);
            }
            "TabItemControl" => {
                let name = py::clean_one_line(&c.name().ok()?);
                if !name.is_empty() && is_selected(c) {
                    let line = format!("[Tab \"{name}\": selected]");
                    if last_text.as_deref() != Some(line.as_str()) {
                        push!(line.clone(), line);
                    }
                }
            }
            "ButtonControl" | "HyperlinkControl" => {
                let name = py::clean_one_line(&c.name().ok()?);
                if !name.is_empty() && !is_icon_glyphs(&name) && last_text.as_deref() != Some(name.as_str()) {
                    let line = if ct == "ButtonControl" { format!("[Button \"{name}\"]") } else { name.clone() };
                    push!(line, name);
                }
            }
            _ => {}
        }
        Some(())
    };
    let mut step = |c: &Ctl, lines: &mut Vec<String>| -> bool {
        visited += 1;
        if visited > 15000 {
            stopped_early = Some("node limit");
            return false;
        }
        if now() > deadline {
            stopped_early = Some("time limit");
            return false;
        }
        visit(c, lines);
        true
    };
    if !tree.is_empty() {
        for (c, _) in &tree {
            if !step(c, &mut lines) {
                break;
            }
        }
    } else {
        for (c, _) in ctx.walk(&root, false, UI_WALK_DEPTH) {
            if !step(&c, &mut lines) {
                break;
            }
        }
    }
    if lines.is_empty() {
        let (val, is_pw) = control_value_info(&root, 50000, true);
        if !val.is_empty() && !is_pw {
            lines.push(val);
        }
    }
    let full_text = lines.join("\n");
    let start = py::int(&a.or("offset", json!(0)))?.max(0) as usize;
    let cap = py::int(&a.or("max_chars", json!(12000)))?.clamp(200, 13000) as usize;
    let total = py::len(&full_text);
    let chunk = py::slice(&full_text, start, start + cap);
    let chunk_len = py::len(chunk);
    let url_part = page_url.map(|u| format!(" url={u}")).unwrap_or_default();
    let head = format!(
        "Window \"{}\" window_id={hwnd}{url_part} [chars {start}-{} of {total}]:",
        py::prefix(&title, 60),
        start + chunk_len
    );
    let body = if !chunk.is_empty() {
        chunk
    } else if full_text.is_empty() {
        "(no readable text found)"
    } else {
        "(empty range; offset past end)"
    };
    let mut foot: Vec<String> = Vec::new();
    if start + chunk_len < total {
        foot.push(format!(
            "... ({} more chars; continue with offset={})",
            total - (start + chunk_len),
            start + chunk_len
        ));
    }
    if let Some(why) = stopped_early {
        foot.push(format!("... (walk stopped at {why} after {visited} controls)"));
    }
    let tail = if foot.is_empty() { String::new() } else { format!("\n{}", foot.join("\n")) };
    Ok(format!("{head}\n{body}{tail}").into())
}

/// Find the listed element again.
fn refind(ctx: &Rc<Ctx>, info: &UiInfo, tree: Option<&Tree>) -> Res<Option<Ctl>> {
    let Some(root) = w(ctx.control_from_handle(info.hwnd))? else { return Ok(None) };
    let mut best: Option<(Ctl, i64)> = None;
    let mut n = 0usize;
    let mut look = |c: &Ctl| -> bool {
        n += 1;
        if n > UI_WALK_LIMIT {
            return false;
        }
        let d = (|| -> Option<i64> {
            if c.type_name().ok()? != info.ctype || py::clean_one_line(&c.name().ok()?) != info.name {
                return None;
            }
            let r = c.rect().ok()?;
            Some(((r.xcenter() - info.x).abs() + (r.ycenter() - info.y).abs()) as i64)
        })();
        if let Some(d) = d {
            if best.as_ref().map(|b| d < b.1).unwrap_or(true) {
                best = Some((c.clone(), d));
            }
        }
        true
    };
    match tree {
        Some(tree) => {
            for (c, _) in tree {
                if !look(c) {
                    break;
                }
            }
        }
        None => {
            for (c, _) in ctx.walk(&root, false, UI_WALK_DEPTH) {
                if !look(&c) {
                    break;
                }
            }
        }
    }
    Ok(best.map(|(c, _)| c.live()))
}

fn ui_info(id: Option<&Value>, name: Option<&Value>, window_id: Option<&Value>) -> Res<(i64, UiInfo)> {
    let mut id: Option<Value> = id.filter(|v| !v.is_null()).cloned();
    if let Some(name) = name.filter(|v| !v.is_null()) {
        // Matching by name avoids the model mixing up id numbers.
        let wanted = py::py_str(name);
        let items: Vec<(i64, UiInfo)> = state::with(|s| {
            s.ui.iter().filter(|(_, v)| CLICKABLE.contains(&v.ctype.as_str())).map(|(i, v)| (*i, v.clone())).collect()
        });
        let mut hits: Vec<&(i64, UiInfo)> = items.iter().filter(|(_, v)| v.name == wanted).collect();
        if hits.is_empty() {
            hits = items.iter().filter(|(_, v)| v.name.contains(&wanted)).collect();
        }
        if hits.is_empty() {
            return Err(Error::tool(format!("No element named \"{wanted}\" in the last ui_elements result.")));
        }
        if hits.len() > 1 {
            let names: Vec<String> =
                hits.iter().take(8).map(|(i, v)| format!("[{i}]{}", py::prefix(&v.name, 20))).collect();
            return Err(Error::tool(format!(
                "\"{wanted}\" matches several elements: {}. Use a fuller name or the id.",
                names.join(", ")
            )));
        }
        id = Some(json!(hits[0].0));
    }
    let Some(id) = id else { return Err(Error::tool("Provide id or name")) };
    let key = py::int(&id)?;
    let Some(info) = state::with(|s| s.ui.get(&key).cloned()) else {
        return Err(Error::tool(format!("No element with id {}; run ui_elements first.", py::py_str(&id))));
    };
    if let Some(wid) = window_id.filter(|v| !v.is_null()) {
        if info.hwnd != py::int(wid)? as isize {
            return Err(Error::tool("The element is not in the given window_id; run ui_elements again."));
        }
    }
    Ok((key, info))
}

fn validate(info: &UiInfo) -> Res<String> {
    win::validate_ui_window(info.hwnd, &info.title, info.pid)
}

fn activate_and_refind(ctx: &Rc<Ctx>, info: &UiInfo, focus_err: &str) -> Res<(i32, i32)> {
    if info.hwnd != 0 {
        win::activate(info.hwnd, false)?;
    }
    validate(info)?;
    let c = refind(ctx, info, None)?;
    if let Some(c) = &c {
        if w(c.is_offscreen())? {
            scroll_into_view(c);
        }
    }
    let usable = match &c {
        Some(c) => !w(c.is_offscreen())? && w(c.is_enabled())?,
        None => false,
    };
    let Some(c) = c.filter(|_| usable) else {
        return Err(Error::tool("The element is gone after switching windows; run ui_elements again."));
    };
    let r = w(c.rect())?;
    if r.width() <= 0 || r.height() <= 0 {
        return Err(Error::tool("The element has no visible area; run ui_elements again."));
    }
    let (x, y) = (r.xcenter(), r.ycenter());
    if win::foreground() != info.hwnd {
        return Err(Error::tool(focus_err));
    }
    state::with(|s| s.focus_point = Some((x, y)));
    Ok((x, y))
}

fn format_text_diff(before: &[String], after: &[String], limit: usize) -> String {
    if before.is_empty() {
        return String::new();
    }
    let added: Vec<&str> = after.iter().filter(|t| !before.contains(t)).take(limit).map(|s| s.as_str()).collect();
    let removed: Vec<&str> = before.iter().filter(|t| !after.contains(t)).take(limit).map(|s| s.as_str()).collect();
    let mut parts: Vec<String> = Vec::new();
    if !added.is_empty() {
        parts.push(format!("+{}", added.join(" | ")));
    }
    if !removed.is_empty() {
        parts.push(format!("-{}", removed.join(" | ")));
    }
    parts.join(" ; ")
}

fn label_of(id: i64, info: &UiInfo, title: &str) -> String {
    format!(
        "[{id}] \"{}\" (window \"{}\", window_id={})",
        py::prefix(&info.name, 40),
        py::prefix(title, 40),
        info.hwnd
    )
}

/// The listed element if it can be used right now (scrolled into view when needed).
fn usable(ctx: &Rc<Ctx>, info: &UiInfo, tree: Option<&Tree>) -> Res<Option<Ctl>> {
    let Some(c) = refind(ctx, info, tree)? else { return Ok(None) };
    if w(c.is_offscreen())? {
        scroll_into_view(&c);
    }
    if w(c.is_offscreen())? || !w(c.is_enabled())? {
        return Ok(None);
    }
    Ok(Some(c))
}

const GONE: &str = "The element is gone, hidden or disabled; run ui_elements again. Old coordinates were not clicked.";

/// Click one listed element. Returns (message, the window's controls as read
/// after the click); a click sequence hands that read to its next step as tree.
fn click_element(
    id: Option<&Value>, name: Option<&Value>, button: &Value, clicks: &Value, window_id: Option<&Value>,
    restore_focus: bool, tree: Option<Tree>,
) -> Res<(String, Option<Tree>)> {
    let (id, info) = ui_info(id, name, window_id)?;
    let title = validate(&info)?;
    let ctx = uia::ctx()?;
    let label = label_of(id, &info, &title);
    let mut tree = tree;
    let mut c: Option<Ctl> = None;
    if tree.is_some() {
        // Read just after the previous click; if the window moved on since, read it again.
        c = usable(&ctx, &info, tree.as_ref()).unwrap_or(None);
    }
    if c.is_none() {
        tree = window_tree(&ctx, info.hwnd); // the texts and the element come from the same moment
        c = usable(&ctx, &info, tree.as_ref())?;
    }
    let Some(c) = c else { return Err(Error::tool(GONE)) };
    let before_texts = window_texts(&ctx, info.hwnd, 12, tree.as_ref()).unwrap_or_default();
    let r = w(c.rect())?;
    state::with(|s| s.focus_point = Some((r.xcenter(), r.ycenter())));
    let mut done: Option<String> = None;
    // Trigger the control directly: works even when the window is covered
    // and does not move the user's mouse.
    if button.as_str() == Some("left") && py::int(clicks)? == 1 {
        for pattern in [uia::INVOKE, uia::SELECTION_ITEM, uia::TOGGLE] {
            if let Ok(true) = c.act(pattern) {
                done = Some(format!("Activated element {label} (directly, without moving the mouse)"));
                break;
            }
        }
    }
    let direct = done.is_some();
    let mut done = match done {
        Some(d) => d,
        None => {
            // Fall back to a real mouse click, with the element's window in front first.
            with_restore_focus(restore_focus, Some(info.hwnd), || {
                let (mx, my) = activate_and_refind(&ctx, &info, "The target window lost focus; click stopped.")?;
                input::click(mx, my, py::int(clicks)?, 0.08, &py::py_str(button))
            })?;
            let suffix = if restore_focus { " (restored previous window)" } else { "" };
            format!("Clicked element {label} with the mouse{suffix}")
        }
    };
    let after = match (&tree, fast_wait()) {
        // watch the controls instead of sleeping: the limit is what the fixed waits add up to
        (Some(before), true) => settle_tree(&ctx, info.hwnd, before, if direct { 0.87 } else { 0.4 }),
        _ => {
            sleep(0.4);
            window_tree(&ctx, info.hwnd)
        }
    };
    // the window may have closed (e.g. a close button)
    if let Ok(texts) = window_texts(&ctx, info.hwnd, 12, after.as_ref()) {
        if !texts.is_empty() {
            done.push_str(&format!("\nWindow text now: {}", texts.join(" | ")));
        }
        let diff = format_text_diff(&before_texts, &texts, 6);
        if !diff.is_empty() {
            done.push_str(&format!("\nWindow diff: {diff}"));
        }
    }
    Ok((done, after))
}

pub fn t_click_element(a: &Args) -> Res<ToolOut> {
    let (text, _) = click_element(
        a.get("id"),
        a.get("name"),
        &a.or("button", json!("left")),
        &a.or("clicks", json!(1)),
        a.get("window_id"),
        a.truthy("restore_focus", false),
        None,
    )?;
    Ok(text.into())
}

pub fn t_type_element(a: &Args) -> Res<ToolOut> {
    let (id, info) = ui_info(a.get("id"), a.get("name"), a.get("window_id"))?;
    let title = validate(&info)?;
    let ctx = uia::ctx()?;
    let label = label_of(id, &info, &title);
    if usable(&ctx, &info, None)?.is_none() {
        return Err(Error::tool(GONE));
    }
    let text = py::py_str(a.req("text"));
    let (clear, enter) = (a.truthy("clear", true), a.truthy("enter", false));
    let restore_focus = a.truthy("restore_focus", false);
    with_restore_focus(restore_focus, Some(info.hwnd), || {
        let (x, y) = activate_and_refind(&ctx, &info, "The target window lost focus; typing stopped.")?;
        input::click(x, y, 1, 0.08, "left")?;
        sleep(0.15);
        if clear {
            input::hotkey(&["ctrl", "a"])?;
            sleep(0.05);
            input::press("backspace")?;
            sleep(0.05);
        }
        if !text.is_empty() {
            type_text(&text)?;
        }
        if enter {
            input::press("enter")?;
        }
        Ok(())
    })?;
    sleep(0.25);
    let mut val_note = String::new();
    if let Ok(Some(c2)) = refind(&ctx, &info, None) {
        let (val, is_pw) = control_value_info(&c2, 300, true);
        if is_pw {
            val_note = "; value now: \"[password]\"".to_string();
        } else if !val.is_empty() {
            let mut v_disp = show_value(&val);
            if py::len(&v_disp) > 200 {
                v_disp = format!("{}…", py::prefix(&v_disp, 200));
            }
            val_note = format!("; value now: \"{v_disp}\"");
        } else if c2.type_name().unwrap_or("") == "EditControl" {
            val_note = "; value now: \"\"".to_string();
        }
    }
    Ok(format!(
        "Typed {} character(s) into {label}{}{}{val_note}",
        py::len(&text),
        if enter { " and pressed Enter" } else { "" },
        if restore_focus { " (restored previous window)" } else { "" }
    )
    .into())
}

/// Run a reviewed, finite list of observed controls in one pinned window.
pub fn t_click_elements(a: &Args) -> Res<ToolOut> {
    let window_id = a.req("window_id");
    let names: Vec<&str> = match a.req("names") {
        Value::Array(list) if (1..=16).contains(&list.len()) => {
            list.iter().filter_map(|n| n.as_str().filter(|s| !s.is_empty())).collect()
        }
        _ => Vec::new(),
    };
    if names.is_empty() || Some(names.len()) != a.req("names").as_array().map(|l| l.len()) {
        return Err(Error::tool("names must be an array of 1-16 button names."));
    }
    // Validate the whole plan before its first action. call_tool holds LOCK so
    // another client cannot replace the remembered elements in the middle of this sequence.
    let mut plan: Vec<(i64, UiInfo)> = Vec::new();
    for name in &names {
        plan.push(ui_info(None, Some(&json!(name)), Some(window_id))?);
    }
    validate(&plan[0].1)?;
    let mut result = String::new();
    let mut tree: Option<Tree> = None;
    for (index, (id, _)) in plan.iter().enumerate() {
        match click_element(Some(&json!(id)), None, &json!("left"), &json!(1), Some(window_id), false, tree.take()) {
            Ok((text, after)) => {
                result = text;
                tree = after;
            }
            Err(exc) => {
                return Err(Error::tool(format!(
                    "Click sequence stopped at step {}/{}; {} step(s) completed: {}",
                    index + 1,
                    plan.len(),
                    index,
                    exc.py_str()
                )));
            }
        }
    }
    Ok(format!("Clicked {} buttons in order: {}\n{result}", plan.len(), names.join(" → ")).into())
}

fn wheel_scroll(amt: i64) -> Res<()> {
    let ticks = amt.abs().min(30);
    let step = if amt > 0 { 120 } else { -120 };
    for i in 0..ticks {
        input::scroll(step)?;
        if i + 1 < ticks {
            sleep(0.02);
        }
    }
    Ok(())
}

fn rect_center_in_window(ctrl: &Ctl, hwnd: Option<isize>) -> Option<(i32, i32)> {
    let r = ctrl.rect().ok()?;
    if r.width() <= 0 || r.height() <= 0 {
        return None;
    }
    let (mut cx, mut cy) = (r.xcenter(), r.ycenter());
    if let Some(wr) = hwnd.filter(|h| *h != 0).and_then(win::window_rect) {
        if wr.right > wr.left && wr.bottom > wr.top {
            cx = (wr.left + 1).max(cx.min(wr.right - 2));
            cy = (wr.top + 1).max(cy.min(wr.bottom - 2));
        }
    }
    Some((cx, cy))
}

fn pct(v: f64) -> i64 {
    py::round(v)
}

/// What the scroll-by-amount attempt came to: a finished message, or "use the plain message".
enum Wheel {
    Message(String),
    Plain,
}

pub fn t_scroll(a: &Args) -> Res<ToolOut> {
    let (amount, percent) = (a.opt("amount"), a.opt("percent"));
    let (x, y) = (a.opt("x"), a.opt("y"));
    let window_id = a.opt("window_id");
    if amount.is_none() && percent.is_none() {
        return Err(Error::tool("Provide amount or percent."));
    }
    let activate = |window_id: &Value| -> Res<isize> {
        let wn = win::find_window(None, Some(window_id))?;
        win::activate(wn.hwnd, wn.hidden)?;
        Ok(wn.hwnd)
    };
    if let Some(percent) = percent {
        let target = py::float(percent)?.clamp(0.0, 100.0);
        let hwnd = match window_id {
            Some(id) => activate(id)?,
            None => win::foreground(),
        };
        let point = match (x, y) {
            (Some(x), Some(y)) => Some(to_real(x, y)?),
            _ => None,
        };
        let ctx = uia::ctx()?;
        let (ctrl, sp) = match point {
            Some((rx, ry)) => find_scrollable_at(&ctx, rx, ry, Some(hwnd))?,
            None => find_scrollable_in_window(&ctx, Some(hwnd), None)?,
        };
        let Some(sp) = sp else {
            return Err(Error::tool("No vertically scrollable UI container found; try scroll with amount and x, y."));
        };
        let (before_pct, mut view_size) = sp.percent();
        w(sp.set_vertical_percent(target))?;
        if fast_wait() {
            settle_percent(&sp, before_pct, Some(target), 0.67);
        } else {
            sleep(0.2);
        }
        let (after_pct, view2) = sp.percent();
        if view2.is_some() {
            view_size = view2;
        }
        let pt = ctrl.as_ref().and_then(|c| rect_center_in_window(c, Some(hwnd)));
        let focus = pt.or(point).unwrap_or_else(|| win::window_center(hwnd));
        state::with(|s| s.focus_point = Some(focus));
        let shown = pct(after_pct.unwrap_or(target));
        let view_note = view_size.map(|v| format!(", view {}%", pct(v))).unwrap_or_default();
        return Ok(match before_pct {
            Some(b) => format!("Scrolled container to {shown}% (was {}%{view_note})", pct(b)),
            None => format!("Scrolled container to {shown}%{view_note}"),
        }
        .into());
    }

    let amount = amount.unwrap_or(&Value::Null);
    let amt = py::int(amount)?;
    if amt == 0 {
        return Err(Error::tool("amount must be non-zero (positive = up, negative = down)."));
    }
    let direction = if amt > 0 { "up" } else { "down" };
    let hwnd: Option<isize> = match window_id {
        Some(id) => Some(activate(id)?),
        None => None,
    };
    let given_point = matches!((x, y), (Some(_), Some(_)));
    let (mut rx, mut ry, move_mouse) = match (x, y) {
        (Some(x), Some(y)) => {
            let (px, py_) = to_real(x, y)?;
            (px, py_, true)
        }
        _ => match hwnd {
            Some(h) => {
                let (px, py_) = win::window_center(h);
                (px, py_, true)
            }
            None => {
                let (px, py_) = input::position();
                (px, py_, false)
            }
        },
    };

    let (mut moved, mut wheeled) = (false, false);
    let attempt = |rx: &mut i32, ry: &mut i32, moved: &mut bool, wheeled: &mut bool| -> Res<Wheel> {
        let ctx = uia::ctx()?;
        let (mut ctrl, mut sp): (Option<Ctl>, Option<ScrollPat>) = (None, None);
        if !given_point && hwnd.is_some() {
            (ctrl, sp) = find_scrollable_in_window(&ctx, hwnd, None)?;
            if let Some(pt) = ctrl.as_ref().and_then(|c| rect_center_in_window(c, hwnd)) {
                (*rx, *ry) = pt;
            }
        }
        state::with(|s| s.focus_point = Some((*rx, *ry)));
        if move_mouse {
            input::move_to(*rx, *ry, 0.0)?;
            *moved = true;
            sleep(0.1);
        }
        if sp.is_none() {
            (ctrl, sp) = find_scrollable_at(&ctx, *rx, *ry, hwnd)?;
        }
        let _ = ctrl;
        let (mut before_pct, mut view_size) = (None, None);
        if let Some(sp) = &sp {
            (before_pct, view_size) = sp.percent();
        }
        *wheeled = true;
        wheel_scroll(amt)?;
        let (Some(sp), Some(before)) = (sp, before_pct) else { return Ok(Wheel::Plain) };
        sleep(0.15);
        let (mut after_pct, view2) = sp.percent();
        if view2.is_some() {
            view_size = view2;
        }
        if amt < 0 && before >= 99.5 && after_pct.map(|v| v >= 99.5).unwrap_or(true) {
            return Ok(Wheel::Message("Container is already at the bottom (100%)".to_string()));
        }
        if amt > 0 && before <= 0.5 && after_pct.map(|v| v <= 0.5).unwrap_or(true) {
            return Ok(Wheel::Message("Container is already at the top (0%)".to_string()));
        }
        let unchanged = |after: Option<f64>| after.map(|v| (v - before).abs() < 0.1).unwrap_or(false);
        let mut via = "";
        if unchanged(after_pct) {
            let (scroll_amt, repeats) = if amt.abs() >= 3 {
                (
                    if amt > 0 { ScrollAmount_LargeDecrement } else { ScrollAmount_LargeIncrement },
                    (amt.abs() / 3).clamp(1, 5),
                )
            } else {
                (if amt > 0 { ScrollAmount_SmallDecrement } else { ScrollAmount_SmallIncrement }, amt.abs())
            };
            for _ in 0..repeats {
                w(sp.scroll_vertical(scroll_amt))?;
            }
            sleep(0.15);
            let (after2, view2) = sp.percent();
            after_pct = after2;
            if view2.is_some() {
                view_size = view2;
            }
            if unchanged(after_pct) {
                let base_step = (view_size.filter(|v| *v != 0.0).unwrap_or(25.0) * 0.8).clamp(10.0, 80.0);
                let step_pct = (base_step * (amt.abs() as f64 / 5.0).clamp(1.0, 2.0)).min(90.0);
                let target_pct = (before + if amt > 0 { -step_pct } else { step_pct }).clamp(0.0, 100.0);
                w(sp.set_vertical_percent(target_pct))?;
                sleep(0.15);
                let (after3, view3) = sp.percent();
                after_pct = after3;
                if view3.is_some() {
                    view_size = view3;
                }
            }
            if after_pct.map(|v| (v - before).abs() >= 0.1).unwrap_or(false) {
                via = " (via UIA scroll)";
            }
        }
        let view_note = view_size.map(|v| format!(", view {}%", pct(v))).unwrap_or_default();
        match after_pct.filter(|v| (v - before).abs() >= 0.1) {
            Some(after) => Ok(Wheel::Message(format!(
                "Scrolled {amt} ({direction}){via}; container scroll: {}% -> {}%{view_note}",
                pct(before),
                pct(after)
            ))),
            None => Ok(Wheel::Message(format!(
                "Scrolled {amt} ({direction}), but scroll position stayed at {}%. Try scroll with x, y over the scrollable area, scroll {{percent}}, or read_window.",
                pct(before)
            ))),
        }
    };
    match attempt(&mut rx, &mut ry, &mut moved, &mut wheeled) {
        Ok(Wheel::Message(m)) => return Ok(m.into()),
        Ok(Wheel::Plain) => {}
        Err(Error::FailSafe) => return Err(Error::FailSafe),
        Err(_) => {
            state::with(|s| s.focus_point = Some((rx, ry)));
            if !wheeled {
                if move_mouse && !moved {
                    input::move_to(rx, ry, 0.0)?;
                    sleep(0.1);
                }
                wheel_scroll(amt)?;
            }
        }
    }
    Ok(format!("Scrolled {amt} (positive = up, negative = down)").into())
}

fn window_has_element(ctx: &Rc<Ctx>, hwnd: isize, query_low: &str) -> Res<Option<String>> {
    let Some(root) = w(ctx.control_from_handle(hwnd))? else { return Ok(None) };
    let (mut n, deadline) = (0, now() + 4.0);
    for (c, _) in ctx.walk(&root, false, 40) {
        n += 1;
        if n > 4000 || now() > deadline {
            break;
        }
        let hit = (|| -> Option<String> {
            if c.is_offscreen().ok()? {
                return None;
            }
            let name = py::clean_one_line(&c.name().ok()?);
            (!name.is_empty() && name.to_lowercase().contains(query_low)).then_some(name)
        })();
        if hit.is_some() {
            return Ok(hit);
        }
    }
    Ok(None)
}

/// Poll until a window, UIA element, window text, or OCR text appears (or disappears if gone=true).
pub fn t_wait_for(a: &Args) -> Res<ToolOut> {
    let (window, window_id) = (a.opt("window"), a.opt("window_id"));
    let (text, element) = (a.opt("text"), a.opt("element"));
    if window.is_none() && window_id.is_none() && text.is_none() && element.is_none() {
        return Err(Error::tool("Provide at least one of window, window_id, text, or element."));
    }
    let (gone, use_ocr) = (a.truthy("gone", false), a.truthy("use_ocr", false));
    let wait_limit = py::float(&a.or("timeout", json!(10)))?.clamp(0.1, 30.0);
    let step = py::float(&a.or("interval", json!(0.5)))?.clamp(0.1, 2.0);
    let started = now();
    let deadline = started + wait_limit;
    loop {
        let mut detail: Option<String> = None;
        if text.is_none() && element.is_none() {
            let wins = win::top_windows();
            let hit = match window_id {
                Some(id) => {
                    let id = py::int(id)? as isize;
                    wins.iter().find(|wn| wn.hwnd == id)
                }
                None => {
                    let q = window.map(py::py_str).unwrap_or_default().to_lowercase();
                    wins.iter().find(|wn| wn.title.to_lowercase().contains(&q))
                }
            };
            if let Some(hit) = hit {
                detail = Some(format!("window \"{}\" (window_id={})", py::prefix(&hit.title, 60), hit.hwnd));
            }
        } else {
            let hwnd = if window.is_some() || window_id.is_some() {
                match win::find_window(window, window_id) {
                    Ok(wn) => Some(wn.hwnd),
                    Err(Error::Tool(_)) => None,
                    Err(e) => return Err(e),
                }
            } else {
                Some(win::foreground())
            };
            if let Some(hwnd) = hwnd.filter(|h| *h != 0) {
                if let Some(element) = element {
                    let ctx = uia::ctx()?;
                    if let Some(name) = window_has_element(&ctx, hwnd, &py::py_str(element).to_lowercase())? {
                        detail = Some(format!("element \"{}\" in window_id={hwnd}", py::prefix(&name, 60)));
                    }
                } else if let Some(text) = text {
                    let q_low = py::py_str(text).to_lowercase();
                    if use_ocr {
                        let (img, _, _) = win::capture_window_image(hwnd)?;
                        let (_, items) = ocr::ocr_image(&img, 15.0)?;
                        if let Some(it) = items.iter().find(|it| it.text.to_lowercase().contains(&q_low)) {
                            detail = Some(format!("OCR text \"{}\" in window_id={hwnd}", py::prefix(&it.text, 60)));
                        }
                    } else {
                        let ctx = uia::ctx()?;
                        let texts = window_texts(&ctx, hwnd, 60, None)?;
                        if let Some(t) = texts.iter().find(|t| t.to_lowercase().contains(&q_low)) {
                            detail = Some(format!("text \"{}\" in window_id={hwnd}", py::prefix(t, 60)));
                        }
                    }
                }
            }
        }
        let describe = || match (element.filter(|v| py::truthy(v)), text.filter(|v| py::truthy(v))) {
            (Some(e), _) => format!("element \"{}\"", py::py_str(e)),
            (None, Some(t)) => format!("text \"{}\"", py::py_str(t)),
            _ => format!("window \"{}\"", window.map(py::py_str).unwrap_or_else(|| "None".to_string())),
        };
        let found = detail.is_some();
        if found != gone {
            let elapsed = now() - started;
            let what = detail.filter(|d| !d.is_empty()).unwrap_or_else(describe);
            let state_word = if gone { "disappeared" } else { "appeared" };
            return Ok(format!("Condition met in {elapsed:.1}s: {what} {state_word}.").into());
        }
        let rem = deadline - now();
        if rem <= 0.0 {
            let state_word = if gone { "disappear" } else { "appear" };
            return Err(Error::tool(format!(
                "Timed out after {wait_limit:.1}s waiting for {} to {state_word}.",
                describe()
            )));
        }
        sleep(step.min(rem));
    }
}

pub const BATCH_GUI_TOOLS: &[&str] = &[
    "click_element", "type_element", "click_text", "click", "double_click", "right_click", "key", "type",
    "scroll", "wait", "wait_for", "focus_window", "ui_elements", "ocr_window", "read_window",
];

/// Run 1-16 validated GUI steps in one turn (with optional single focus-restore at the end).
pub fn t_gui_batch(a: &Args) -> Res<ToolOut> {
    let window_id = a.opt("window_id");
    let restore_focus = a.truthy("restore_focus", false);
    let steps = match a.req("steps") {
        Value::Array(list) if (1..=16).contains(&list.len()) => list,
        _ => return Err(Error::tool("steps must be an array of 1-16 step objects.")),
    };
    let mut plan: Vec<(&'static tools::ToolDef, Map<String, Value>)> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let idx = i + 1;
        let Value::Object(step) = step else {
            return Err(Error::tool(format!("Step {idx} must be a JSON object.")));
        };
        let action = match step.get("action").filter(|v| py::truthy(v)) {
            Some(v) => v.clone(),
            None => step.get("tool").cloned().unwrap_or(Value::Null),
        };
        let def = action.as_str().filter(|n| BATCH_GUI_TOOLS.contains(n)).and_then(tools::find);
        let Some(def) = def else {
            let mut allowed: Vec<&str> = BATCH_GUI_TOOLS.to_vec();
            allowed.sort();
            return Err(Error::tool(format!(
                "Step {idx} has unsupported action {}. Allowed: {}",
                py::repr(&action),
                allowed.join(", ")
            )));
        };
        let mut sargs: Map<String, Value> = match step.get("args") {
            Some(Value::Object(args)) => args.clone(),
            Some(_) => return Err(Error::tool(format!("Step {idx} args must be a JSON object."))),
            None => step.iter().filter(|(k, _)| *k != "action" && *k != "tool").map(|(k, v)| (k.clone(), v.clone())).collect(),
        };
        sargs.shift_remove("screenshot");
        let has = |p: &str| def.params.iter().any(|(n, _)| *n == p);
        if let Some(wid) = window_id {
            if has("window_id") && !sargs.contains_key("window_id") {
                sargs.insert("window_id".to_string(), wid.clone());
            }
        }
        if def.name == "ocr_window" && !sargs.contains_key("overlay") {
            sargs.insert("overlay".to_string(), json!(false));
        }
        // When restore_focus is requested on the batch, save/restore once around the whole sequence.
        if restore_focus && has("restore_focus") && !sargs.contains_key("restore_focus") {
            sargs.insert("restore_focus".to_string(), json!(false));
        }
        plan.push((def, sargs));
    }
    let target_hwnd = opt_hwnd(a)?;
    let mut outputs: Vec<String> = Vec::new();
    with_restore_focus(restore_focus, target_hwnd, || {
        for (i, (def, sargs)) in plan.iter().enumerate() {
            match tools::invoke(def, sargs) {
                Ok(out) => {
                    let first_line = if out.text.is_empty() {
                        "ok".to_string()
                    } else {
                        py::splitlines(&out.text).first().copied().unwrap_or("").to_string()
                    };
                    outputs.push(format!("{}. {}: {}", i + 1, def.name, py::prefix(&first_line, 180)));
                }
                Err(exc) => {
                    return Err(Error::tool(format!(
                        "gui_batch stopped at step {}/{} ({}); {} step(s) completed: {}",
                        i + 1,
                        plan.len(),
                        def.name,
                        i,
                        exc.py_str()
                    )));
                }
            }
        }
        Ok(())
    })?;
    let suffix = if restore_focus { " (restored previous window)" } else { "" };
    Ok(format!("Completed {} step(s){suffix}:\n{}", plan.len(), outputs.join("\n")).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(hwnd: isize, ctype: &str, name: &str) -> UiInfo {
        UiInfo { hwnd, title: "Calculator".into(), pid: 30, ctype: ctype.into(), name: name.into(), x: 10, y: 10 }
    }

    #[test]
    fn text_diff_lists_added_and_removed() {
        let before = vec!["0".to_string(), "Standard".to_string()];
        let after = vec!["Display is 5".to_string(), "Standard".to_string()];
        assert_eq!(format_text_diff(&before, &after, 6), "+Display is 5 ; -0");
        assert_eq!(format_text_diff(&[], &after, 6), "");
        assert_eq!(format_text_diff(&before, &before, 6), "");
    }

    #[test]
    fn icon_glyph_names_are_recognised() {
        assert!(is_icon_glyphs(""));
        assert!(is_icon_glyphs("\u{e700} \u{f8ff}"));
        assert!(!is_icon_glyphs("OK"));
        assert_eq!(show_value("a\r\nb\rc\nd"), "a\\nb\\nc\\nd");
    }

    #[test]
    fn ui_info_resolves_names_ids_and_windows() {
        let _guard = state::tool_lock();
        state::with(|s| {
            s.ui.clear();
            s.ui.insert(1, info(12, "ButtonControl", "Seven"));
            s.ui.insert(2, info(12, "ButtonControl", "Equals"));
            s.ui.insert(3, info(12, "TextControl", "Equals"));
            s.ui.insert(4, info(12, "ButtonControl", "Seven"));
            s.ui.insert(5, info(12, "ButtonControl", "Clear entry"));
        });
        // a TextControl with the same text does not shadow the button
        assert_eq!(ui_info(None, Some(&json!("Equals")), None).unwrap().0, 2);
        // a substring is enough when nothing matches exactly
        assert_eq!(ui_info(None, Some(&json!("Clear")), None).unwrap().0, 5);
        // duplicate exact names need the id
        let err = ui_info(None, Some(&json!("Seven")), None).unwrap_err();
        assert_eq!(
            err,
            Error::tool("\"Seven\" matches several elements: [1]Seven, [4]Seven. Use a fuller name or the id.")
        );
        assert_eq!(
            ui_info(None, Some(&json!("Nine")), None).unwrap_err(),
            Error::tool("No element named \"Nine\" in the last ui_elements result.")
        );
        // the element must belong to the pinned window
        assert_eq!(
            ui_info(Some(&json!(1)), None, Some(&json!(99))).unwrap_err(),
            Error::tool("The element is not in the given window_id; run ui_elements again.")
        );
        assert_eq!(ui_info(Some(&json!("2")), None, Some(&json!(12))).unwrap().0, 2);
        assert_eq!(ui_info(None, None, None).unwrap_err(), Error::tool("Provide id or name"));
        assert_eq!(
            ui_info(Some(&json!(77)), None, None).unwrap_err(),
            Error::tool("No element with id 77; run ui_elements first.")
        );
        state::with(|s| s.ui.clear());
    }
}
