"""GeminiForge local tool server.

Runs on 127.0.0.1 only. The Chrome extension sends tool calls that Gemini
wrote in gemini.google.com, this server runs them on this computer and
returns the result (text, and a screenshot for computer-use tools).
"""
import base64
import ctypes
import io
import json
import os
import re
import subprocess
import sys
import threading
import time
import traceback
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# Must run before pyautogui/mss so mouse coordinates match real screen pixels
# on displays with Windows scaling (125%, 150%...).
if sys.platform == "win32":
    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(2)
    except Exception:
        try:
            ctypes.windll.user32.SetProcessDPIAware()
        except Exception:
            pass

import mss  # noqa: E402
import pyautogui  # noqa: E402
import pyperclip  # noqa: E402
from PIL import Image, ImageDraw, ImageFont  # noqa: E402

import backups  # noqa: E402
import language  # noqa: E402

HOST = "127.0.0.1"
PORT = int(os.environ.get("GF_PORT", "18765"))
MAX_OUTPUT = 15000          # characters of text sent back to Gemini per tool
SHOT_MAX_WIDTH = 1366       # screenshots are scaled down to this width
SKIP_DIRS = {".git", "node_modules", "__pycache__", ".venv", "venv", "dist", "build", ".next"}

pyautogui.FAILSAFE = True   # slam the mouse into a screen corner to abort
pyautogui.PAUSE = 0.05

STATE = {
    "cwd": str(Path.home()),
    "shot_scale": 1.0,      # real pixels per screenshot pixel
    "shot_origin": (0, 0),  # monitor top-left in real pixels
    "shot_size": None,      # (w, h) of the last screenshot, None before the first
    "ui": {},               # element id -> real (x, y) from the last ui_elements
}
LOCK = threading.Lock()
# uiautomation owns a process-wide COM client. Keep it on one living thread;
# HTTP/AnyIO workers can otherwise create it in a thread that later goes away.
UI_EXECUTOR = ThreadPoolExecutor(max_workers=1, thread_name_prefix="GeminiForgeUI")


class ToolError(Exception):
    pass


# ---------------------------------------------------------------- helpers

def resolve(path):
    if not path:
        return Path(STATE["cwd"])
    p = Path(os.path.expandvars(os.path.expanduser(str(path))))
    if not p.is_absolute():
        p = Path(STATE["cwd"]) / p
    return p.resolve()


def clip(text):
    if len(text) <= MAX_OUTPUT:
        return text
    half = MAX_OUTPUT // 2
    return (text[:half] + f"\n\n...[{len(text) - MAX_OUTPUT} characters truncated]...\n\n" + text[-half:])


def is_binary(p):
    try:
        with open(p, "rb") as f:
            return b"\0" in f.read(4096)
    except OSError:
        return True


def read_text(p):
    data = p.read_bytes()
    for enc in ("utf-8", "utf-8-sig", "cp950", "gbk"):
        try:
            return data.decode(enc)
        except UnicodeDecodeError:
            continue
    return data.decode("utf-8", errors="replace")


def to_real(x, y):
    s = STATE["shot_scale"]
    ox, oy = STATE["shot_origin"]
    return int(round(ox + float(x) * s)), int(round(oy + float(y) * s))


def list_monitors():
    """Physical monitors, primary first, as dicts with left/top/width/height."""
    mk = getattr(mss, "MSS", None) or mss.mss
    with mk() as sct:
        mons = [dict(m) for m in sct.monitors[1:]]
    mons.sort(key=lambda m: (not m.get("is_primary", m["left"] == 0 and m["top"] == 0), m["left"], m["top"]))
    return mons


def take_screenshot(grid=True, monitor=None, point=None):
    """Capture one monitor: the given number, else the one holding `point`
    (e.g. the window just acted on), else the one with the mouse."""
    mons = list_monitors()
    mx, my = pyautogui.position()
    px, py = point or (mx, my)
    if monitor:
        idx = max(1, min(int(monitor), len(mons)))
    else:
        idx = next((i for i, m in enumerate(mons, 1)
                    if m["left"] <= px < m["left"] + m["width"] and m["top"] <= py < m["top"] + m["height"]), 1)
    mon = mons[idx - 1]
    mk = getattr(mss, "MSS", None) or mss.mss
    with mk() as sct:
        raw = sct.grab({k: mon[k] for k in ("left", "top", "width", "height")})
        img = Image.frombytes("RGB", raw.size, raw.bgra, "raw", "BGRX")
    scale = 1.0
    if img.width > SHOT_MAX_WIDTH:
        scale = img.width / SHOT_MAX_WIDTH
        img = img.resize((SHOT_MAX_WIDTH, round(img.height / scale)), Image.LANCZOS)
    STATE["shot_scale"] = scale
    STATE["shot_origin"] = (mon["left"], mon["top"])
    STATE["shot_size"] = img.size

    # draw the mouse position so Gemini can see where it is
    mx, my = pyautogui.position()
    cx, cy = (mx - mon["left"]) / scale, (my - mon["top"]) / scale
    d = ImageDraw.Draw(img, "RGBA")
    if grid:
        try:
            font = ImageFont.truetype("arial.ttf", 11)
        except OSError:
            font = ImageFont.load_default()
        step = 100
        for gx in range(0, img.width, step):
            d.line([(gx, 0), (gx, img.height)], fill=(255, 0, 255, 60), width=1)
        for gy in range(0, img.height, step):
            d.line([(0, gy), (img.width, gy)], fill=(255, 0, 255, 60), width=1)
        for gx in range(0, img.width, step * 2):
            for gy in range(0, img.height, step * 2):
                label = f"{gx},{gy}"
                d.rectangle([gx + 1, gy + 1, gx + 7 * len(label), gy + 13], fill=(0, 0, 0, 110))
                d.text((gx + 2, gy + 1), label, fill=(255, 255, 0, 255), font=font)
    d.ellipse([cx - 6, cy - 6, cx + 6, cy + 6], outline=(255, 0, 0, 255), width=2)

    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=82)
    others = ", ".join(f"{i}={m['width']}x{m['height']}" + (" (primary)" if i == 1 else "") for i, m in enumerate(mons, 1))
    inside = 0 <= cx < img.width and 0 <= cy < img.height
    note = (f"Monitor {idx} screenshot {img.width}x{img.height} (coordinates are pixels of this image; purple grid lines every 100px, labeled every 200px; "
            + (f"red circle = mouse at {round(cx)},{round(cy)}" if inside else "mouse is not on this monitor")
            + f"). Monitors: {others}. Switch with screenshot {{monitor: N}}.")
    return note, base64.b64encode(buf.getvalue()).decode()


def to_shot(rx, ry):
    """Real screen pixels -> last screenshot pixels, or None if off that screenshot."""
    if not STATE["shot_size"]:
        return None
    s = STATE["shot_scale"]
    ox, oy = STATE["shot_origin"]
    x, y = (rx - ox) / s, (ry - oy) / s
    w, h = STATE["shot_size"]
    return (round(x), round(y)) if 0 <= x < w and 0 <= y < h else None


def type_text(text):
    if text.isascii() and "\n" not in text:
        pyautogui.write(text, interval=0.01)
        return
    old = None
    try:
        old = pyperclip.paste()
    except Exception:
        pass
    pyperclip.copy(text)
    time.sleep(0.1)
    pyautogui.hotkey("ctrl", "v")
    time.sleep(0.3)
    if old is not None:
        try:
            pyperclip.copy(old)
        except Exception:
            pass


def parse_keys(keys):
    parts = [k.strip().lower() for k in re.split(r"[+]", str(keys)) if k.strip()]
    alias = {"control": "ctrl", "windows": "win", "cmd": "win", "return": "enter",
             "escape": "esc", "del": "delete", "pgup": "pageup", "pgdn": "pagedown"}
    return [alias.get(p, p) for p in parts]


# ---------------------------------------------------------------- tools

def t_cd(path):
    p = resolve(path)
    if not p.is_dir():
        raise ToolError(f"Folder not found: {p}")
    STATE["cwd"] = str(p)
    return f"Current folder: {p}"


def t_list_dir(path=".", depth=1, directories_only=False):
    root = resolve(path)
    if not root.is_dir():
        raise ToolError(f"Folder not found: {root}")
    depth = max(1, min(int(depth), 4))
    lines = [f"{root}"]
    count = 0

    def walk(d, level, prefix):
        nonlocal count
        try:
            entries = d.iterdir()
            if directories_only:
                entries = (e for e in entries if e.is_dir())
            entries = sorted(entries, key=lambda e: (not e.is_dir(), e.name.lower()))
        except PermissionError:
            lines.append(prefix + "[access denied]")
            return
        for e in entries:
            count += 1
            if count > 500:
                lines.append(prefix + "... (too many entries, rest omitted)")
                return
            if e.is_dir():
                lines.append(f"{prefix}{e.name}/")
                if level < depth and e.name not in SKIP_DIRS:
                    walk(e, level + 1, prefix + "  ")
            else:
                try:
                    size = e.stat().st_size
                except OSError:
                    size = 0
                lines.append(f"{prefix}{e.name}  ({size:,} B)")

    walk(root, 1, "  ")
    return "\n".join(lines)


def t_read_file(path, offset=1, limit=400):
    p = resolve(path)
    if not p.is_file():
        raise ToolError(f"File not found: {p}")
    if is_binary(p):
        raise ToolError(f"Binary file, cannot read as text: {p}")
    lines = read_text(p).splitlines()
    offset = max(1, int(offset))
    limit = max(1, min(int(limit), 2000))
    chunk = lines[offset - 1: offset - 1 + limit]
    body = "\n".join(f"{i:>5}\t{l}" for i, l in enumerate(chunk, start=offset))
    more = ""
    if offset - 1 + limit < len(lines):
        more = f"\n... ({len(lines)} lines total; continue with offset={offset + limit})"
    return f"{p} ({len(lines)} lines)\n{body}{more}"


def t_write_file(path, content):
    p = resolve(path)
    if p.is_dir():
        raise ToolError(f"This is a folder, not a file: {p}")
    p.parent.mkdir(parents=True, exist_ok=True)
    _, note = backups.snapshot(p, "write_file")
    p.write_text(content, encoding="utf-8", newline="")
    return f"Wrote {p} ({len(content)} characters, {content.count(chr(10)) + 1} lines){note}"


def t_edit_file(path, old, new, replace_all=False):
    p = resolve(path)
    if not p.is_file():
        raise ToolError(f"File not found: {p}")
    text = read_text(p)
    n = text.count(old)
    if n == 0:
        # tolerate CRLF files when Gemini sends LF
        if "\r\n" in text and text.count(old.replace("\n", "\r\n")):
            old, new = old.replace("\n", "\r\n"), new.replace("\n", "\r\n")
            n = text.count(old)
    if n == 0:
        raise ToolError("Text `old` not found (it must match the file exactly, including spaces and indentation). Check with read_file first.")
    if n > 1 and not replace_all:
        raise ToolError(f"`old` appears {n} times; add more context to make it unique, or set replace_all=true.")
    text = text.replace(old, new) if replace_all else text.replace(old, new, 1)
    _, note = backups.snapshot(p, "edit_file")
    p.write_text(text, encoding="utf-8", newline="")
    return f"Edited {p} ({n if replace_all else 1} replacement(s)){note}"


def t_list_backups(limit=20, path=None):
    return backups.list_backups(limit, str(resolve(path)) if path else None)


def t_restore_backup(backup_id):
    try:
        return backups.restore(backup_id)
    except ValueError as e:
        raise ToolError(str(e))


def t_search(pattern, path=".", glob="*", ignore_case=True):
    root = resolve(path)
    try:
        rx = re.compile(pattern, re.IGNORECASE if ignore_case else 0)
    except re.error as e:
        raise ToolError(f"Invalid regular expression: {e}")
    files = [root] if root.is_file() else root.rglob(glob)
    out, hits = [], 0
    for f in files:
        if any(part in SKIP_DIRS for part in f.parts) or not f.is_file():
            continue
        try:
            if f.stat().st_size > 2_000_000 or is_binary(f):
                continue
            for i, line in enumerate(read_text(f).splitlines(), 1):
                if rx.search(line):
                    out.append(f"{f}:{i}: {line.strip()[:200]}")
                    hits += 1
                    if hits >= 200:
                        out.append("... (over 200 matches; narrow the search)")
                        return "\n".join(out)
        except OSError:
            continue
    return "\n".join(out) if out else "No matches found"


def t_find_files(pattern, path="."):
    root = resolve(path)
    out = []
    for f in root.rglob(pattern):
        if any(part in SKIP_DIRS for part in f.parts):
            continue
        out.append(str(f))
        if len(out) >= 300:
            out.append("... (over 300 files)")
            break
    return "\n".join(out) if out else "No matching files found"


# Gemini gives up on a tool call after about a minute. run waits at most this
# long, then leaves the command running as a job that job_status can check.
RUN_WAIT = 40
JOBS = {}  # job id -> dict(proc, out, err, cmd, cwd, started, deadline, killed)


def _kill_tree(pid):
    subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], capture_output=True,
                   creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))


def _job_text(jid, job, tail=None):
    dec = lambda path: Path(path).read_bytes().decode("utf-8", errors="replace") if Path(path).exists() else ""
    out, err = dec(job["out"]), dec(job["err"])
    if tail and len(out) > tail:
        out = "… (earlier output omitted)\n" + out[-tail:]
    code = job["proc"].poll()
    secs = int(time.time() - job["started"])
    if code is None:
        head = f"[job {jid} running for {secs}s; force-stopped in at most {int(job['deadline'] - time.time())}s]"
    elif job["killed"]:
        why = "stopped by stop_job" if job["killed"] == "stopped" else "stopped after timeout"
        head = f"[job {jid} {why}] [exit code {code}]"
    else:
        head = f"[job {jid} finished in {secs}s] [exit code {code}]"
    text = f"{head} [folder {job['cwd']}]\n{out}"
    if err.strip():
        text += f"\n[stderr]\n{err[-4000:]}"
    return text


def _watchdog(jid):
    job = JOBS[jid]
    try:
        job["proc"].wait(timeout=max(0, job["deadline"] - time.time()))
    except subprocess.TimeoutExpired:
        job["killed"] = True
        _kill_tree(job["proc"].pid)


def t_run(command, cwd=None, timeout=120):
    workdir = resolve(cwd) if cwd else Path(STATE["cwd"])
    timeout = max(1, min(int(timeout), 900))
    ps = ("[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;"
          "$OutputEncoding=[System.Text.Encoding]::UTF8;" + command)
    jid = max(JOBS, default=0) + 1
    base = Path(os.environ.get("TEMP", ".")) / f"geminiforge_job{jid}_{int(time.time())}"
    out, err = f"{base}.out.log", f"{base}.err.log"
    with open(out, "wb") as fo, open(err, "wb") as fe:
        proc = subprocess.Popen(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", ps],
            cwd=str(workdir), stdin=subprocess.DEVNULL, stdout=fo, stderr=fe,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
    now = time.time()
    JOBS[jid] = {"proc": proc, "out": out, "err": err, "cmd": command, "cwd": str(workdir),
                 "started": now, "deadline": now + timeout, "killed": False}
    threading.Thread(target=_watchdog, args=(jid,), daemon=True).start()
    try:
        proc.wait(timeout=min(timeout, RUN_WAIT))
    except subprocess.TimeoutExpired:
        pass
    if proc.poll() is None and timeout <= RUN_WAIT:
        JOBS[jid]["killed"] = True
        _kill_tree(proc.pid)
        proc.wait()
    if proc.poll() is None:
        return (_job_text(jid, JOBS[jid], tail=3000) +
                f"\n\nThe command is still running in the background (job {jid}, PID {proc.pid}). "
                f"Use job_status {{id: {jid}, seconds: 30}} to wait for it and get the output; stop it with stop_job {{id: {jid}}}.")
    return _job_text(jid, JOBS[jid])


def t_job_status(id=None, seconds=0):
    if id is None:
        if not JOBS:
            return "No jobs started by run."
        return "\n".join(f"{j}: {'running' if v['proc'].poll() is None else 'finished'}  {v['cmd'][:120]}"
                         for j, v in sorted(JOBS.items(), reverse=True)[:20])
    job = JOBS.get(int(id))
    if not job:
        raise ToolError(f"No job {id}. Call job_status without id to list all jobs.")
    try:
        job["proc"].wait(timeout=max(0, min(float(seconds), RUN_WAIT)))
    except subprocess.TimeoutExpired:
        pass
    return _job_text(int(id), job, tail=MAX_OUTPUT - 2000)


def t_stop_job(id):
    job = JOBS.get(int(id))
    if not job:
        raise ToolError(f"No job {id}.")
    if job["proc"].poll() is None:
        job["killed"] = "stopped"
        _kill_tree(job["proc"].pid)
        job["proc"].wait(timeout=10)
    return _job_text(int(id), job, tail=3000)


def t_start_process(command, cwd=None):
    workdir = resolve(cwd) if cwd else Path(STATE["cwd"])
    log = Path(os.environ.get("TEMP", ".")) / f"geminiforge_{int(time.time())}.log"
    f = open(log, "wb")
    p = subprocess.Popen(
        ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command],
        cwd=str(workdir), stdout=f, stderr=subprocess.STDOUT,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    return f"Started in the background (PID {p.pid}). Output is logged to {log}; view it with read_file."


def t_open(target):
    os.startfile(os.path.expandvars(str(target)))
    time.sleep(1.5)
    return f"Opened: {target}"


def t_screenshot(grid=True, monitor=None):
    return take_screenshot(grid, monitor)


def t_click(x, y, button="left", clicks=1):
    rx, ry = to_real(x, y)
    pyautogui.click(rx, ry, clicks=int(clicks), interval=0.08, button=button)
    return f"Clicked {button} button {clicks}x at ({x},{y})"


def t_double_click(x, y):
    return t_click(x, y, clicks=2)


def t_right_click(x, y):
    return t_click(x, y, button="right")


def t_move(x, y):
    pyautogui.moveTo(*to_real(x, y), duration=0.2)
    return f"Mouse moved to ({x},{y})"


def t_drag(x1, y1, x2, y2):
    pyautogui.moveTo(*to_real(x1, y1))
    pyautogui.dragTo(*to_real(x2, y2), duration=0.5, button="left")
    return f"Dragged from ({x1},{y1}) to ({x2},{y2})"


def t_type(text, enter=False):
    type_text(str(text))
    if enter:
        pyautogui.press("enter")
    return f"Typed {len(str(text))} characters" + (" and pressed Enter" if enter else "")


def t_key(keys):
    ks = parse_keys(keys)
    if len(ks) == 1:
        pyautogui.press(ks[0])
    else:
        pyautogui.hotkey(*ks)
    return f"Pressed {'+'.join(ks)}"


def t_scroll(amount, x=None, y=None):
    if x is not None and y is not None:
        pyautogui.moveTo(*to_real(x, y))
    pyautogui.scroll(int(amount) * 120)
    return f"Scrolled {amount} (positive = up, negative = down)"


def t_wait(seconds=1):
    time.sleep(max(0, min(float(seconds), 30)))
    return f"Waited {seconds} seconds"


def t_clipboard_get():
    return pyperclip.paste()


def t_clipboard_set(text):
    pyperclip.copy(str(text))
    return "Copied to clipboard"


# ---------------------------------------------------------------- windows / UI automation
# Lets Gemini click buttons by name instead of guessing pixel positions.

CLICKABLE = {"ButtonControl", "MenuItemControl", "ListItemControl", "TreeItemControl", "TabItemControl",
             "HyperlinkControl", "EditControl", "CheckBoxControl", "RadioButtonControl", "ComboBoxControl",
             "SplitButtonControl", "DataItemControl", "DocumentControl"}


def _auto():
    import uiautomation as auto
    return auto


def _top_windows():
    """Visible top-level windows with a title, including minimized ones."""
    user32, dwm = ctypes.windll.user32, ctypes.windll.dwmapi
    from ctypes import wintypes
    out = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def cb(hwnd, _):
        if not user32.IsWindowVisible(hwnd):
            return True
        n = user32.GetWindowTextLengthW(hwnd)
        if not n:
            return True
        cloaked = ctypes.c_int(0)
        dwm.DwmGetWindowAttribute(hwnd, 14, ctypes.byref(cloaked), ctypes.sizeof(cloaked))  # DWMWA_CLOAKED
        # 2 = hidden by the shell (e.g. a freshly launched Store app such as
        # Calculator, or one on another virtual desktop): keep it, marked hidden.
        if cloaked.value and cloaked.value != 2:
            return True
        ex = user32.GetWindowLongW(hwnd, -20)  # GWL_EXSTYLE
        if ex & 0x80 and not ex & 0x40000:      # tool window, not app window
            return True
        buf = ctypes.create_unicode_buffer(n + 1)
        user32.GetWindowTextW(hwnd, buf, n + 1)
        title = buf.value.strip()
        if title in ("Program Manager",) or not title:
            return True
        r = wintypes.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(r))
        minimized = bool(user32.IsIconic(hwnd))
        if not minimized and (r.right - r.left < 80 or r.bottom - r.top < 40):
            return True
        out.append({"hwnd": hwnd, "title": title, "min": minimized, "hidden": bool(cloaked.value),
                    "rect": (r.left, r.top, r.right - r.left, r.bottom - r.top)})
        return True

    user32.EnumWindows(cb, 0)
    return out


def _find_window(title=None, window_id=None):
    if window_id is not None:
        wins = [w for w in _top_windows() if w["hwnd"] == int(window_id)]
        if not wins or (title and wins[0]["title"].casefold() != str(title).casefold()):
            raise ToolError("That window is gone or its title changed; run list_windows / ui_elements again.")
        return wins[0]
    if not title or not str(title).strip():
        raise ToolError("Provide a window title or window_id.")
    t = str(title).lower()
    wins = [w for w in _top_windows() if t in w["title"].lower()]
    if not wins:
        raise ToolError(f"No window title contains \"{title}\". Use list_windows to see open windows.")
    # Best match first: exact title, then title starting with it, then the
    # shortest title. Otherwise "Calculator" could match a browser tab titled
    # "Calculator automation test - Google Chrome" instead of Calculator itself.
    wins.sort(key=lambda w: (w["title"].lower() != t, not w["title"].lower().startswith(t),
                             len(w["title"]), w["hidden"]))
    return wins[0]


def _activate(win):
    user32 = ctypes.windll.user32
    hwnd = win["hwnd"]
    if user32.IsIconic(hwnd) or win.get("hidden"):
        user32.ShowWindow(hwnd, 9)  # SW_RESTORE (also brings a hidden Store app back)
    # Windows only lets the foreground process change focus; a tap of Alt lifts that lock.
    pyautogui.press("alt")
    user32.SetForegroundWindow(hwnd)
    time.sleep(0.5)
    if user32.GetForegroundWindow() != hwnd:
        raise ToolError("Could not bring the target window to the front; stopped to avoid acting on another program.")


def t_list_windows():
    fg = ctypes.windll.user32.GetForegroundWindow()
    lines = []
    for w in _top_windows()[:60]:
        x, y, wd, ht = w["rect"]
        state = ("(hidden; bring it back with focus_window, or use ui_elements/click_element directly)" if w["hidden"]
                 else "(minimized)" if w["min"] else f"at ({x},{y}) size {wd}x{ht}")
        mark = " <- active" if w["hwnd"] == fg else ""
        lines.append(f"- window_id={w['hwnd']} \"{w['title'][:80]}\" {state}{mark}")
    return "Visible windows:\n" + "\n".join(lines)


def _window_center(hwnd):
    from ctypes import wintypes
    r = wintypes.RECT()
    ctypes.windll.user32.GetWindowRect(hwnd, ctypes.byref(r))
    return (r.left + r.right) // 2, (r.top + r.bottom) // 2


def t_focus_window(title=None, window_id=None):
    w = _find_window(title, window_id)
    _activate(w)
    STATE["focus_point"] = _window_center(w["hwnd"])
    return f"Switched to window \"{w['title'][:80]}\""


def _window_texts(hwnd, limit=12):
    """Short visible texts in a window, so Gemini can check a result without a screenshot."""
    auto = _auto()
    root = auto.ControlFromHandle(hwnd)
    out, n = [], 0
    if not root:
        return out
    for c, _ in auto.WalkControl(root, includeTop=False, maxDepth=40):
        n += 1
        if n > 4000 or len(out) >= limit:
            break
        try:
            if c.ControlTypeName == "TextControl" and not c.IsOffscreen:
                t = (c.Name or "").strip().replace("\n", " ")
                # skip icon-font glyphs (private use area), which read as blanks
                if all("" <= ch <= "" or ch.isspace() for ch in t):
                    continue
                if len(t) <= 60 and t not in out:
                    out.append(t)
        except Exception:
            continue
    return out


def t_ui_elements(window=None, filter=None, max=150, include_text=False, window_id=None):
    STATE["ui"] = {}  # a failed observation must not leave earlier elements usable
    auto = _auto()
    with auto.UIAutomationInitializerInThread():
        if window or window_id is not None:
            w = _find_window(window, window_id)
            if w["min"] or w["hidden"]:
                _activate(w)  # hidden/minimized windows expose no buttons until shown
            target = auto.ControlFromHandle(w["hwnd"])
        else:
            target = auto.GetForegroundControl()
        if not target:
            raise ToolError("No active window found")
        hwnd = target.NativeWindowHandle
        title = target.Name or ""
        if (window or window_id is not None) and (hwnd != w["hwnd"] or title != w["title"]):
            raise ToolError("The window changed while being read; run list_windows again. No other program was read instead.")
        process_id = _window_process_id(hwnd)
        f = str(filter).lower() if filter else None
        limit = max_items = int(max)
        items, visited, deadline = [], 0, time.time() + 10
        for c, _depth in auto.WalkControl(target, includeTop=False, maxDepth=40):
            visited += 1
            if visited > 6000 or time.time() > deadline or len(items) >= max_items:
                break
            try:
                ct = c.ControlTypeName
                if ct not in CLICKABLE and not (include_text and ct == "TextControl"):
                    continue
                name = (c.Name or "").strip().replace("\n", " ")
                if not name and ct != "EditControl":
                    continue
                if f and f not in name.lower():
                    continue
                r = c.BoundingRectangle
                if r.width() <= 0 or r.height() <= 0 or c.IsOffscreen:
                    continue
                items.append((ct, name, r.xcenter(), r.ycenter()))
            except Exception:
                continue
        STATE["ui"] = {i: {"hwnd": hwnd, "title": title, "pid": process_id,
                           "type": ct, "name": name, "x": x, "y": y}
                       for i, (ct, name, x, y) in enumerate(items, 1)}
        lines = []
        for i, (ct, name, x, y) in enumerate(items, 1):
            ct = ct.replace("Control", "")
            pos = to_shot(x, y)
            where = (f"screenshot ≈{pos[0]},{pos[1]}" if pos
                     else "not on the last screenshot's monitor" if STATE["shot_size"] else "")
            lines.append(f"[{i}] {ct} \"{name[:70]}\" {where}")
        head = (f"Elements in window \"{title[:60]}\" window_id={hwnd}"
                f" ({len(items)} found; click with click_element {{id, window_id}}):")
        if not items:
            return head + "\n(none found; try another filter, include_text=true, or click by screenshot coordinates)"
        if len(items) >= limit:
            lines.append(f"... (only the first {limit} are listed; narrow with filter)")
        return head + "\n" + "\n".join(lines)


def _refind(info):
    """Find the listed element again (fresh COM object for this thread)."""
    auto = _auto()
    root = auto.ControlFromHandle(info["hwnd"])
    if not root:
        return None
    best, best_d, n = None, None, 0
    for c, _ in auto.WalkControl(root, includeTop=False, maxDepth=40):
        n += 1
        if n > 6000:
            break
        try:
            if c.ControlTypeName != info["type"] or (c.Name or "").strip().replace("\n", " ") != info["name"]:
                continue
            r = c.BoundingRectangle
            d = abs(r.xcenter() - info["x"]) + abs(r.ycenter() - info["y"])
            if best is None or d < best_d:
                best, best_d = c, d
        except Exception:
            continue
    return best


def _window_process_id(hwnd):
    pid = ctypes.c_ulong()
    ctypes.windll.user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def _ui_info(id=None, name=None, window_id=None):
    if name is not None:
        # Matching by name avoids the model mixing up id numbers.
        items = [(i, v) for i, v in STATE["ui"].items() if v["type"] in CLICKABLE]
        hits = [(i, v) for i, v in items if v["name"] == str(name)] or \
               [(i, v) for i, v in items if str(name) in v["name"]]
        if not hits:
            raise ToolError(f"No element named \"{name}\" in the last ui_elements result.")
        if len(hits) > 1:
            names = ", ".join(f"[{i}]{v['name'][:20]}" for i, v in hits[:8])
            raise ToolError(f"\"{name}\" matches several elements: {names}. Use a fuller name or the id.")
        id = hits[0][0]
    if id is None:
        raise ToolError("Provide id or name")
    info = STATE["ui"].get(int(id))
    if not info:
        raise ToolError(f"No element with id {id}; run ui_elements first.")
    if window_id is not None and info["hwnd"] != int(window_id):
        raise ToolError("The element is not in the given window_id; run ui_elements again.")
    return int(id), info


def _validate_ui_window(info):
    user32 = ctypes.windll.user32
    n = user32.GetWindowTextLengthW(info["hwnd"])
    buf = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(info["hwnd"], buf, n + 1)
    if (not user32.IsWindow(info["hwnd"]) or buf.value != info["title"]
            or _window_process_id(info["hwnd"]) != info["pid"]):
        raise ToolError("The original window closed or changed; run ui_elements again. Old coordinates were not clicked.")
    return buf.value


def t_click_element(id=None, name=None, button="left", clicks=1, window_id=None):
    id, info = _ui_info(id, name, window_id)
    title = _validate_ui_window(info)
    auto = _auto()
    label = f"[{id}] \"{info['name'][:40]}\" (window \"{title[:40]}\", window_id={info['hwnd']})"
    with auto.UIAutomationInitializerInThread():
        c = _refind(info)
        if not c or c.IsOffscreen or not c.IsEnabled:
            raise ToolError("The element is gone, hidden or disabled; run ui_elements again. Old coordinates were not clicked.")
        r = c.BoundingRectangle
        x, y = r.xcenter(), r.ycenter()
        STATE["focus_point"] = (x, y)
        done = None
        # Trigger the control directly: works even when the window is covered
        # and does not move the user's mouse.
        if c and button == "left" and int(clicks) == 1:
            for pid, act in ((auto.PatternId.InvokePattern, "Invoke"),
                             (auto.PatternId.SelectionItemPattern, "Select"),
                             (auto.PatternId.TogglePattern, "Toggle")):
                try:
                    pat = c.GetPattern(pid)
                    if pat:
                        getattr(pat, act)()
                        done = f"Activated element {label} (directly, without moving the mouse)"
                        break
                except Exception:
                    continue
    if not done:
        # Fall back to a real mouse click, with the element's window in front first.
        if info["hwnd"]:
            _activate({"hwnd": info["hwnd"]})
        _validate_ui_window(info)
        with auto.UIAutomationInitializerInThread():
            c = _refind(info)
            if not c or c.IsOffscreen or not c.IsEnabled:
                raise ToolError("The element is gone after switching windows; run ui_elements again.")
            r = c.BoundingRectangle
            x, y = r.xcenter(), r.ycenter()
        if ctypes.windll.user32.GetForegroundWindow() != info["hwnd"]:
            raise ToolError("The target window lost focus; click stopped.")
        pyautogui.click(x, y, clicks=int(clicks), interval=0.08, button=button)
        done = f"Clicked element {label} with the mouse"
    time.sleep(0.4)
    try:
        with auto.UIAutomationInitializerInThread():
            texts = _window_texts(info["hwnd"])
        if texts:
            done += "\nWindow text now: " + " | ".join(texts)
    except Exception:
        pass  # the window may have closed (e.g. a close button)
    return done


def t_click_elements(window_id, names):
    """Run a reviewed, finite list of observed controls in one pinned window."""
    if not isinstance(names, list) or not 1 <= len(names) <= 16 or any(
            not isinstance(name, str) or not name for name in names):
        raise ToolError("names must be an array of 1-16 button names.")
    # Validate the whole plan before its first action. call_tool holds LOCK so
    # another client cannot replace STATE['ui'] in the middle of this sequence.
    plan = [_ui_info(name=name, window_id=window_id) for name in names]
    _validate_ui_window(plan[0][1])
    result = ""
    for index, (id, _) in enumerate(plan, 1):
        try:
            result = t_click_element(id=id, window_id=window_id)
        except Exception as exc:
            raise ToolError(f"Click sequence stopped at step {index}/{len(plan)}; {index - 1} step(s) completed: {exc}") from exc
    return f"Clicked {len(plan)} buttons in order: {' → '.join(names)}\n{result}"


def t_zoom(x1, y1, x2, y2):
    """Close-up of part of the last screenshot; does not change click coordinates."""
    if not STATE["shot_size"]:
        raise ToolError("Run screenshot first")
    (rx1, ry1), (rx2, ry2) = to_real(x1, y1), to_real(x2, y2)
    left, top = min(rx1, rx2), min(ry1, ry2)
    w, h = abs(rx2 - rx1), abs(ry2 - ry1)
    if w < 10 or h < 10:
        raise ToolError("Area too small")
    mk = getattr(mss, "MSS", None) or mss.mss
    with mk() as sct:
        raw = sct.grab({"left": left, "top": top, "width": w, "height": h})
        img = Image.frombytes("RGB", raw.size, raw.bgra, "raw", "BGRX")
    factor = min(3.0, SHOT_MAX_WIDTH / img.width, 1200 / img.height)
    if factor > 1:
        img = img.resize((round(img.width * factor), round(img.height * factor)), Image.LANCZOS)
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=88)
    note = (f"Zoomed view of ({x1},{y1})-({x2},{y2}) from the last screenshot. "
            "Note: clicks still use the coordinates of the full screenshot.")
    return note, base64.b64encode(buf.getvalue()).decode()


TOOLS = {
    "cd": t_cd, "list_dir": t_list_dir, "read_file": t_read_file, "write_file": t_write_file,
    "edit_file": t_edit_file, "search": t_search, "find_files": t_find_files, "run": t_run,
    "start_process": t_start_process, "job_status": t_job_status, "stop_job": t_stop_job,
    "open": t_open, "screenshot": t_screenshot,
    "click": t_click, "double_click": t_double_click, "right_click": t_right_click,
    "move": t_move, "drag": t_drag, "type": t_type, "key": t_key, "scroll": t_scroll,
    "wait": t_wait, "clipboard_get": t_clipboard_get, "clipboard_set": t_clipboard_set,
    "list_windows": t_list_windows, "focus_window": t_focus_window, "ui_elements": t_ui_elements,
    "click_element": t_click_element, "click_elements": t_click_elements, "zoom": t_zoom,
    "list_backups": t_list_backups, "restore_backup": t_restore_backup,
}
# These change what is on screen, so a fresh screenshot is returned after them.
SCREEN_ACTIONS = {"click", "double_click", "right_click", "move", "drag", "type", "key", "scroll", "open",
                  "click_element", "click_elements", "focus_window"}
UI_TOOLS = {"ui_elements", "click_element", "click_elements", "focus_window"}


def call_tool(name, args):
    if name in UI_TOOLS:
        return UI_EXECUTOR.submit(_call_tool, name, args).result()
    return _call_tool(name, args)


def _call_tool(name, args):
    fn = TOOLS.get(name)
    if not fn:
        raise ToolError(f"Unknown tool: {name}. Available: {', '.join(TOOLS)}")
    args = dict(args or {})
    want_shot = args.pop("screenshot", True)
    with LOCK:
        STATE["focus_point"] = None
        result = fn(**args)
        image = None
        if isinstance(result, tuple):
            result, image = result
        elif name in SCREEN_ACTIONS and want_shot:
            time.sleep(0.8)
            note, image = take_screenshot(point=STATE.get("focus_point"))
            result = f"{result}\n{note}"
    return clip(str(result)), image


# ---------------------------------------------------------------- http

ALLOWED_HOSTS = {f"127.0.0.1:{PORT}", f"localhost:{PORT}"}


class Handler(BaseHTTPRequestHandler):
    server_version = "GeminiForge/1.0"

    def log_message(self, fmt, *a):
        pass

    def _send(self, code, obj):
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        origin = self.headers.get("Origin", "")
        if origin.startswith("chrome-extension://"):
            self.send_header("Access-Control-Allow-Origin", origin)
        self.end_headers()
        self.wfile.write(body)

    def _allowed(self):
        # Blocks DNS-rebinding and calls from ordinary web pages: only the
        # extension (chrome-extension:// origin) may run tools.
        if self.headers.get("Host", "") not in ALLOWED_HOSTS:
            return False
        origin = self.headers.get("Origin", "")
        return origin.startswith("chrome-extension://")

    def do_OPTIONS(self):
        self._send(403, {"ok": False})

    def do_GET(self):
        if self.path == "/health" and self.headers.get("Host", "") in ALLOWED_HOSTS:
            return self._send(200, {"ok": True, "cwd": STATE["cwd"], "tools": list(TOOLS),
                                    "languageSync": True})
        self._send(404, {"ok": False})

    def do_POST(self):
        if self.path not in ("/tool", "/language") or not self._allowed():
            return self._send(403, {"ok": False, "error": "forbidden"})
        try:
            n = int(self.headers.get("Content-Length", "0"))
            if self.path == "/language":
                if not 0 < n <= 128:
                    return self._send(400, {"ok": False})
                try:
                    preference = json.loads(self.rfile.read(n))
                    language.save_language(preference["language"])
                except (ValueError, TypeError, KeyError):
                    return self._send(400, {"ok": False})
                return self._send(200, {"ok": True})
            req = json.loads(self.rfile.read(n).decode("utf-8"))
            name, args = req.get("tool"), req.get("args") or {}
            print(f"[{time.strftime('%H:%M:%S')}] {name} {json.dumps(args, ensure_ascii=False)[:200]}", flush=True)
            text, image = call_tool(name, args)
            self._send(200, {"ok": True, "output": text, "image": image})
        except ToolError as e:
            self._send(200, {"ok": False, "error": str(e)})
        except TypeError as e:
            self._send(200, {"ok": False, "error": f"Invalid arguments: {e}"})
        except pyautogui.FailSafeException:
            self._send(200, {"ok": False, "error": "The user moved the mouse to a screen corner; emergency stop."})
        except Exception as e:
            traceback.print_exc()
            self._send(200, {"ok": False, "error": f"{type(e).__name__}: {e}"})


def main():
    ThreadingHTTPServer.allow_reuse_address = False  # fail if the port is taken
    try:
        srv = ThreadingHTTPServer((HOST, PORT), Handler)
    except OSError:
        print(language.t("srvPortInUse", port=PORT))
        input(language.t("pressEnterToExit"))
        return
    print(language.t("srvStarted", url=f"http://{HOST}:{PORT}"))
    print(language.t("srvCwd", path=STATE["cwd"]))
    print(language.t("srvStop"))
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
