//! UI Automation access: the parts of the Python `uiautomation` package the
//! tools use, plus `ui_cache.snapshot` (one cross-process read of a subtree).
//!
//! Everything here runs on the single UI thread (see `tools::run_on_ui`).

use crate::py::Error;
use crate::state::{fast_wait, sleep};
use std::cell::RefCell;
use std::rc::Rc;
use windows::core::{Interface, Result as W};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Variant::{VariantClear, VARIANT};
use windows::Win32::UI::Accessibility::*;

/// The `uiautomation` package sleeps this long after Invoke / Select / Toggle /
/// Scroll / SetScrollPercent / ScrollIntoView (its OPERATION_WAIT_TIME).
pub const OPERATION_WAIT_TIME: f64 = 0.5;

/// The pause after a pattern call. With fast waits the callers watch for the
/// effect themselves, so only a short moment is left here.
fn op_wait() {
    sleep(if fast_wait() { 0.03 } else { OPERATION_WAIT_TIME });
}

pub const INVOKE: i32 = 10000;
pub const VALUE: i32 = 10002;
pub const SCROLL: i32 = 10004;
pub const SELECTION_ITEM: i32 = 10010;
pub const TEXT: i32 = 10014;
pub const TOGGLE: i32 = 10015;
pub const SCROLL_ITEM: i32 = 10017;
pub const LEGACY: i32 = 10018;

// UIA property ids
const P_RECT: i32 = 30001;
const P_TYPE: i32 = 30003;
const P_NAME: i32 = 30005;
const P_ENABLED: i32 = 30010;
const P_PASSWORD: i32 = 30019;
const P_OFFSCREEN: i32 = 30022;
const P_TOGGLE_STATE: i32 = 30086;
const P_IS_SELECTED: i32 = 30079;
/// pattern id -> its "is this pattern available" property id
const AVAILABLE: [(i32, i32); 6] =
    [(VALUE, 30043), (SCROLL, 30034), (SELECTION_ITEM, 30036), (TEXT, 30040), (TOGGLE, 30041), (LEGACY, 30090)];
const TREE_SCOPE_ELEMENT: i32 = 1;
const TREE_SCOPE_SUBTREE: i32 = 7; // element | children | descendants

const CONTROL_TYPE_NAMES: [&str; 41] = [
    "ButtonControl", "CalendarControl", "CheckBoxControl", "ComboBoxControl", "EditControl",
    "HyperlinkControl", "ImageControl", "ListItemControl", "ListControl", "MenuControl", "MenuBarControl",
    "MenuItemControl", "ProgressBarControl", "RadioButtonControl", "ScrollBarControl", "SliderControl",
    "SpinnerControl", "StatusBarControl", "TabControl", "TabItemControl", "TextControl", "ToolBarControl",
    "ToolTipControl", "TreeControl", "TreeItemControl", "CustomControl", "GroupControl", "ThumbControl",
    "DataGridControl", "DataItemControl", "DocumentControl", "SplitButtonControl", "WindowControl",
    "PaneControl", "HeaderControl", "HeaderItemControl", "TableControl", "TitleBarControl",
    "SeparatorControl", "SemanticZoomControl", "AppBarControl",
];

pub fn control_type_name(id: i32) -> &'static str {
    usize::try_from(id - 50000).ok().and_then(|i| CONTROL_TYPE_NAMES.get(i)).copied().unwrap_or("")
}

/// A COM failure the way comtypes words it.
pub fn com_error(e: &windows::core::Error) -> Error {
    Error::exc("COMError", format!("({}, '{}', (None, None, None, 0, None))", e.code().0, e.message().trim_end()))
}

fn is_null(e: &windows::core::Error) -> bool {
    e.code() == windows::core::Error::empty().code()
}

/// A method that returned S_OK with a null pointer means "nothing there".
fn opt<T>(r: W<T>) -> W<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if is_null(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

pub struct Ctx {
    pub auto: IUIAutomation,
    pub walker: IUIAutomationTreeWalker,
}

thread_local! {
    static CTX: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

/// Called once on the UI thread before it runs any tool.
pub fn init_thread() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// The automation client of this thread (created on first use, three tries).
pub fn ctx() -> Result<Rc<Ctx>, Error> {
    CTX.with(|cell| {
        if let Some(c) = cell.borrow().as_ref() {
            return Ok(c.clone());
        }
        let mut last = None;
        for _ in 0..3 {
            let made: W<Ctx> = unsafe {
                CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .and_then(|auto| auto.RawViewWalker().map(|walker| Ctx { auto, walker }))
            };
            match made {
                Ok(c) => {
                    let rc = Rc::new(c);
                    *cell.borrow_mut() = Some(rc.clone());
                    return Ok(rc);
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.map(|e| com_error(&e)).unwrap_or_else(|| Error::runtime("UI Automation is not available")))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn xcenter(&self) -> i32 {
        self.left + self.width().div_euclid(2)
    }
    pub fn ycenter(&self) -> i32 {
        self.top + self.height().div_euclid(2)
    }
}

enum VarValue {
    Bool(bool),
    Int(i32),
    Empty,
    Other,
}

fn read_variant(mut v: VARIANT) -> VarValue {
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        let out = match inner.vt.0 {
            11 => VarValue::Bool(inner.Anonymous.boolVal.0 != 0), // VT_BOOL
            3 | 22 | 19 | 23 => VarValue::Int(inner.Anonymous.lVal), // VT_I4 / VT_INT / VT_UI4 / VT_UINT
            2 | 18 => VarValue::Int(inner.Anonymous.iVal as i32),  // VT_I2 / VT_UI2
            0 | 1 => VarValue::Empty,                              // VT_EMPTY / VT_NULL
            _ => VarValue::Other,
        };
        let _ = VariantClear(&mut v);
        out
    }
}

/// One control: an element read live, or one of a cached tree (`ui_cache.Snap`),
/// which answers the properties the tools read from the cache.
#[derive(Clone)]
pub struct Ctl {
    pub el: IUIAutomationElement,
    pub cached: bool,
}

impl Ctl {
    /// The real control behind a cached walk item, for acting on it or re-reading it.
    pub fn live(&self) -> Ctl {
        Ctl { el: self.el.clone(), cached: false }
    }

    pub fn type_id(&self) -> W<i32> {
        unsafe { if self.cached { self.el.CachedControlType() } else { self.el.CurrentControlType() } }.map(|t| t.0)
    }

    /// `ControlTypeName`
    pub fn type_name(&self) -> W<&'static str> {
        self.type_id().map(control_type_name)
    }

    /// `Name` (never None)
    pub fn name(&self) -> W<String> {
        unsafe { if self.cached { self.el.CachedName() } else { self.el.CurrentName() } }.map(|b| b.to_string())
    }

    pub fn is_offscreen(&self) -> W<bool> {
        unsafe { if self.cached { self.el.CachedIsOffscreen() } else { self.el.CurrentIsOffscreen() } }
            .map(|b| b.as_bool())
    }

    pub fn is_enabled(&self) -> W<bool> {
        unsafe { if self.cached { self.el.CachedIsEnabled() } else { self.el.CurrentIsEnabled() } }.map(|b| b.as_bool())
    }

    pub fn is_password(&self) -> W<bool> {
        unsafe { if self.cached { self.el.CachedIsPassword() } else { self.el.CurrentIsPassword() } }
            .map(|b| b.as_bool())
    }

    /// `BoundingRectangle`
    pub fn rect(&self) -> W<Rect> {
        unsafe { if self.cached { self.el.CachedBoundingRectangle() } else { self.el.CurrentBoundingRectangle() } }
            .map(|r| Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
    }

    /// `NativeWindowHandle` (0 on failure), always read live.
    pub fn native_hwnd(&self) -> isize {
        unsafe { self.el.CurrentNativeWindowHandle() }.map(|h| h.0 as isize).unwrap_or(0)
    }

    fn cached_value(&self, prop: i32) -> W<VarValue> {
        unsafe { self.el.GetCachedPropertyValue(UIA_PROPERTY_ID(prop)) }.map(read_variant)
    }

    /// For a cached control: whether the pattern is available according to the cache.
    /// `Ok(true)` for live controls and for patterns the cache does not track.
    fn cache_says_available(&self, pattern_id: i32) -> W<bool> {
        if !self.cached {
            return Ok(true);
        }
        let Some((_, prop)) = AVAILABLE.iter().find(|(p, _)| *p == pattern_id) else { return Ok(true) };
        Ok(match self.cached_value(*prop)? {
            VarValue::Bool(b) => b,
            VarValue::Int(i) => i != 0,
            VarValue::Empty => false,
            VarValue::Other => true,
        })
    }

    /// `GetPattern(pattern_id)`: None when the control does not support it (or the call failed).
    /// `Err` only comes from reading the cached availability.
    pub fn pattern<T: Interface>(&self, pattern_id: i32) -> W<Option<T>> {
        if !self.cache_says_available(pattern_id)? {
            return Ok(None);
        }
        let unk = unsafe { self.el.GetCurrentPattern(UIA_PATTERN_ID(pattern_id)) };
        Ok(unk.ok().and_then(|u| u.cast::<T>().ok()))
    }

    /// TogglePattern.ToggleState as an int, None when there is no such pattern.
    pub fn toggle_state(&self) -> W<Option<i32>> {
        if self.cached {
            if !self.cache_says_available(TOGGLE)? {
                return Ok(None);
            }
            return Ok(match self.cached_value(P_TOGGLE_STATE)? {
                VarValue::Int(i) => Some(i),
                _ => None,
            });
        }
        match self.pattern::<IUIAutomationTogglePattern>(TOGGLE)? {
            Some(p) => unsafe { p.CurrentToggleState() }.map(|s| Some(s.0)),
            None => Ok(None),
        }
    }

    /// SelectionItemPattern.IsSelected, None when there is no such pattern.
    pub fn is_selected(&self) -> W<Option<bool>> {
        if self.cached {
            if !self.cache_says_available(SELECTION_ITEM)? {
                return Ok(None);
            }
            return Ok(Some(match self.cached_value(P_IS_SELECTED)? {
                VarValue::Bool(b) => b,
                VarValue::Int(i) => i != 0,
                VarValue::Empty => false,
                VarValue::Other => true,
            }));
        }
        match self.pattern::<IUIAutomationSelectionItemPattern>(SELECTION_ITEM)? {
            Some(p) => unsafe { p.CurrentIsSelected() }.map(|b| Some(b.as_bool())),
            None => Ok(None),
        }
    }

    pub fn value_pattern_value(&self) -> W<Option<String>> {
        match self.pattern::<IUIAutomationValuePattern>(VALUE)? {
            Some(p) => unsafe { p.CurrentValue() }.map(|b| Some(b.to_string())),
            None => Ok(None),
        }
    }

    pub fn legacy_value(&self) -> W<Option<String>> {
        match self.pattern::<IUIAutomationLegacyIAccessiblePattern>(LEGACY)? {
            Some(p) => unsafe { p.CurrentValue() }.map(|b| Some(b.to_string())),
            None => Ok(None),
        }
    }

    /// TextPattern.DocumentRange.GetText(max_length)
    pub fn document_text(&self, max_length: i32) -> W<Option<String>> {
        match self.pattern::<IUIAutomationTextPattern>(TEXT)? {
            Some(p) => unsafe { p.DocumentRange()?.GetText(max_length) }.map(|b| Some(b.to_string())),
            None => Ok(None),
        }
    }

    pub fn scroll_pattern(&self) -> W<Option<ScrollPat>> {
        Ok(self.pattern::<IUIAutomationScrollPattern>(SCROLL)?.map(ScrollPat))
    }

    /// ScrollItemPattern.ScrollIntoView(); Ok(false) when there is no such pattern.
    pub fn scroll_into_view(&self) -> W<bool> {
        match self.pattern::<IUIAutomationScrollItemPattern>(SCROLL_ITEM)? {
            Some(p) => {
                unsafe { p.ScrollIntoView() }?;
                op_wait();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Invoke / Select / Toggle; Ok(false) when there is no such pattern.
    pub fn act(&self, pattern_id: i32) -> W<bool> {
        let done = match pattern_id {
            INVOKE => match self.pattern::<IUIAutomationInvokePattern>(INVOKE)? {
                Some(p) => unsafe { p.Invoke() }.map(|_| true)?,
                None => false,
            },
            SELECTION_ITEM => match self.pattern::<IUIAutomationSelectionItemPattern>(SELECTION_ITEM)? {
                Some(p) => unsafe { p.Select() }.map(|_| true)?,
                None => false,
            },
            TOGGLE => match self.pattern::<IUIAutomationTogglePattern>(TOGGLE)? {
                Some(p) => unsafe { p.Toggle() }.map(|_| true)?,
                None => false,
            },
            _ => false,
        };
        if done {
            op_wait();
        }
        Ok(done)
    }
}

#[derive(Clone)]
pub struct ScrollPat(pub IUIAutomationScrollPattern);

impl ScrollPat {
    /// `VerticallyScrollable is True`
    pub fn vertically_scrollable(&self) -> bool {
        unsafe { self.0.CurrentVerticallyScrollable() }.map(|b| b.as_bool()).unwrap_or(false)
    }

    /// `_scroll_percent(sp)`: (vertical position, view size), each None when not readable or negative.
    pub fn percent(&self) -> (Option<f64>, Option<f64>) {
        let keep = |r: W<f64>| r.ok().filter(|v| *v >= 0.0);
        unsafe { (keep(self.0.CurrentVerticalScrollPercent()), keep(self.0.CurrentVerticalViewSize())) }
    }

    /// `SetScrollPercent(NoScrollValue, percent)`
    pub fn set_vertical_percent(&self, percent: f64) -> W<()> {
        unsafe { self.0.SetScrollPercent(UIA_ScrollPatternNoScroll, percent) }?;
        op_wait();
        Ok(())
    }

    /// `Scroll(NoAmount, amount)`
    pub fn scroll_vertical(&self, amount: ScrollAmount) -> W<()> {
        unsafe { self.0.Scroll(ScrollAmount_NoAmount, amount) }?;
        op_wait();
        Ok(())
    }
}

/// `Control.CreateControlFromElement`: None for an element whose type is not a known control type.
fn live_ctl(el: IUIAutomationElement) -> W<Option<Ctl>> {
    let id = unsafe { el.CurrentControlType() }?.0;
    Ok((50000..=50040).contains(&id).then_some(Ctl { el, cached: false }))
}

fn live_opt(r: W<IUIAutomationElement>) -> W<Option<Ctl>> {
    match opt(r)? {
        Some(el) => live_ctl(el),
        None => Ok(None),
    }
}

impl Ctx {
    /// `auto.ControlFromHandle(hwnd)`
    pub fn control_from_handle(&self, hwnd: isize) -> W<Option<Ctl>> {
        if hwnd == 0 {
            return Ok(None);
        }
        live_opt(unsafe { self.auto.ElementFromHandle(HWND(hwnd as *mut core::ffi::c_void)) })
    }

    /// `auto.ControlFromPoint(x, y)`
    pub fn control_from_point(&self, x: i32, y: i32) -> W<Option<Ctl>> {
        live_opt(unsafe { self.auto.ElementFromPoint(POINT { x, y }) })
    }

    /// `control.GetParentControl()`
    pub fn parent(&self, c: &Ctl) -> W<Option<Ctl>> {
        live_opt(unsafe { self.walker.GetParentElement(&c.el) })
    }

    fn first_child(&self, c: &Ctl) -> W<Option<Ctl>> {
        live_opt(unsafe { self.walker.GetFirstChildElement(&c.el) })
    }

    fn next_sibling(&self, c: &Ctl) -> W<Option<Ctl>> {
        live_opt(unsafe { self.walker.GetNextSiblingElement(&c.el) })
    }

    fn request(&self, scope: i32) -> W<IUIAutomationCacheRequest> {
        unsafe {
            let request = self.auto.CreateCacheRequest()?;
            for prop in [P_RECT, P_TYPE, P_NAME, P_ENABLED, P_PASSWORD, P_OFFSCREEN, P_TOGGLE_STATE, P_IS_SELECTED] {
                request.AddProperty(UIA_PROPERTY_ID(prop))?;
            }
            for (_, prop) in AVAILABLE {
                request.AddProperty(UIA_PROPERTY_ID(prop))?;
            }
            request.SetTreeScope(TreeScope(scope))?;
            request.SetTreeFilter(&self.auto.RawViewCondition()?)?; // the view WalkControl walks
            Ok(request)
        }
    }

    /// `ui_cache.snapshot`: (control, depth) in the order WalkControl yields, or
    /// None when the tree cannot be cached (the caller then walks it the slow way).
    ///
    /// Field values are deliberately not cached, so a password box's value is
    /// never pulled into this process: values are still read live, after IsPassword.
    pub fn snapshot(&self, root: &Ctl, include_top: bool, max_depth: u32) -> Option<Vec<(Ctl, u32)>> {
        let run = || -> W<Vec<(Ctl, u32)>> {
            unsafe {
                let request = self.request(TREE_SCOPE_SUBTREE)?;
                let mut out = Vec::new();
                if include_top {
                    let top = root.el.BuildUpdatedCache(&self.request(TREE_SCOPE_ELEMENT)?)?;
                    out.push((Ctl { el: top, cached: true }, 0));
                }
                // The root's own children come from sibling steps, not from one read
                // of the whole subtree: asked for a window's subtree, UIA can leave
                // out child windows (File Explorer loses its tab strip and its
                // content pane).
                let mut top_level = Vec::new();
                let mut child = opt(self.walker.GetFirstChildElementBuildCache(&root.el, &request))?;
                while let Some(c) = child {
                    child = opt(self.walker.GetNextSiblingElementBuildCache(&c, &request))?;
                    top_level.push(c);
                }
                let mut stack: Vec<(IUIAutomationElement, u32)> =
                    top_level.into_iter().rev().map(|c| (c, 1)).collect();
                while let Some((element, depth)) = stack.pop() {
                    if depth < max_depth {
                        if let Some(children) = opt(element.GetCachedChildren())? {
                            let n = children.Length()?;
                            for i in (0..n).rev() {
                                stack.push((children.GetElement(i)?, depth + 1));
                            }
                        }
                    }
                    out.push((Ctl { el: element, cached: true }, depth));
                }
                Ok(out)
            }
        };
        run().ok()
    }

    /// `_walk`: the whole subtree in one read when possible, else the slow walk.
    pub fn walk(self: &Rc<Self>, root: &Ctl, include_top: bool, max_depth: u32) -> Walk {
        match self.snapshot(root, include_top, max_depth) {
            Some(list) => Walk::Cached(list.into_iter()),
            None => self.walk_live(root, include_top, max_depth),
        }
    }

    /// `auto.WalkControl(control, includeTop, maxDepth)`
    pub fn walk_live(self: &Rc<Self>, root: &Ctl, include_top: bool, max_depth: u32) -> Walk {
        let root = root.live();
        let list = if max_depth > 0 { vec![self.first_child(&root).ok().flatten()] } else { Vec::new() };
        Walk::Live(LiveWalk { ctx: self.clone(), top: include_top.then_some(root), list, max_depth })
    }
}

pub struct LiveWalk {
    ctx: Rc<Ctx>,
    top: Option<Ctl>,
    /// the current control at every depth, None once a level is exhausted
    list: Vec<Option<Ctl>>,
    max_depth: u32,
}

pub enum Walk {
    Cached(std::vec::IntoIter<(Ctl, u32)>),
    Live(LiveWalk),
}

impl Walk {
    pub fn is_cached(&self) -> bool {
        matches!(self, Walk::Cached(_))
    }
}

impl Iterator for Walk {
    type Item = (Ctl, u32);

    fn next(&mut self) -> Option<(Ctl, u32)> {
        let w = match self {
            Walk::Cached(it) => return it.next(),
            Walk::Live(w) => w,
        };
        if let Some(top) = w.top.take() {
            return Some((top, 0));
        }
        loop {
            let depth = w.list.len().checked_sub(1)?;
            match w.list[depth].clone() {
                Some(last) => {
                    w.list[depth] = w.ctx.next_sibling(&last).ok().flatten();
                    if (depth as u32) + 1 < w.max_depth {
                        if let Some(child) = w.ctx.first_child(&last).ok().flatten() {
                            w.list.push(Some(child));
                        }
                    }
                    return Some((last, depth as u32 + 1));
                }
                None => {
                    w.list.pop();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_type_names_follow_uia_ids() {
        assert_eq!(control_type_name(50000), "ButtonControl");
        assert_eq!(control_type_name(50004), "EditControl");
        assert_eq!(control_type_name(50020), "TextControl");
        assert_eq!(control_type_name(50030), "DocumentControl");
        assert_eq!(control_type_name(50032), "WindowControl");
        assert_eq!(control_type_name(50040), "AppBarControl");
        assert_eq!(control_type_name(49999), "");
        assert_eq!(control_type_name(50041), "");
    }

    #[test]
    fn rect_centers_floor_like_python() {
        let r = Rect { left: 10, top: 20, right: 15, bottom: 27 };
        assert_eq!((r.width(), r.height(), r.xcenter(), r.ycenter()), (5, 7, 12, 23));
        let neg = Rect { left: -7, top: 0, right: -4, bottom: 0 };
        assert_eq!(neg.xcenter(), -6);
    }
}
