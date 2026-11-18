"""GeminiForge MCP server for Gemini's "@ custom app" (MCP connector).

Gemini (gemini.google.com) calls these tools through an https URL. Locally the
server listens on 127.0.0.1:18766; Tailscale Funnel publishes it at
https://<this-pc>.ts.net:10000/mcp.

Security:
- Every MCP request needs an OAuth access token. Getting one requires the
  pairing code shown in this server's window (entered once when connecting).
- Tools that change things (write files, run commands, click...) pop up a
  confirmation window on this PC, unless approval mode says otherwise.
"""
import base64
import hashlib
import hmac
import html
import inspect
import json
import os
import secrets
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Optional
from urllib.parse import urlencode, urlparse

import anyio
import uvicorn
from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import HTMLResponse, JSONResponse, PlainTextResponse, RedirectResponse
from starlette.routing import Route

import server as gf  # the GeminiForge tools (also sets DPI awareness)
from language import current_language, messages_for, t as tr
from mcp.server.mcpserver import Image, MCPServer
from mcp.server.mcpserver.exceptions import ToolError as MCPToolError
from mcp.server.transport_security import TransportSecuritySettings
from mcp.types import ToolAnnotations

HERE = Path(__file__).resolve().parent
CONFIG_FILE = HERE / "mcp_config.json"
STORE_FILE = HERE / "mcp_auth.json"
PORT = int(os.environ.get("GF_MCP_PORT", "18766"))
FUNNEL_PORT = 10000
TAILSCALE = r"C:\Program Files\Tailscale\tailscale.exe"
ACCESS_TTL = 24 * 3600
REFRESH_TTL = 180 * 24 * 3600

READ_ONLY = {"cd", "list_dir", "read_file", "search", "find_files", "screenshot", "wait", "job_status",
             "clipboard_get", "list_windows", "ui_elements", "zoom", "list_backups"}


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


# ---------------------------------------------------------------- config

def detect_public_url():
    exe = TAILSCALE if Path(TAILSCALE).exists() else "tailscale"
    try:
        out = subprocess.run([exe, "status", "--json"], capture_output=True, timeout=10,
                             creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        dns = json.loads(out.stdout)["Self"]["DNSName"].rstrip(".")
        return f"https://{dns}:{FUNNEL_PORT}" if dns else ""
    except Exception:
        return ""


def load_config():
    cfg = {}
    if CONFIG_FILE.exists():
        cfg = json.loads(CONFIG_FILE.read_text(encoding="utf-8"))
    defaults = {
        "public_url": "",
        "pairing_code": f"{secrets.randbelow(10 ** 8):08d}",
        "client_id": "geminiforge",
        "client_secret": secrets.token_urlsafe(32),
        # readonly: confirm everything except read-only tools; all: confirm every tool; none: never confirm
        "approval": "readonly",
        # on: Gemini web asks before non-read-only tools; off: report every tool as read-only
        # so Gemini never asks (your choice -- injected instructions then run unchecked)
        "web_confirm": "on",
    }
    changed = False
    for k, v in defaults.items():
        if k not in cfg:
            cfg[k] = v
            changed = True
    if not cfg["public_url"]:
        cfg["public_url"] = detect_public_url()
        changed = changed or bool(cfg["public_url"])
    if changed:
        CONFIG_FILE.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")
    return cfg


CFG = load_config()


def public_url(request=None):
    if CFG["public_url"]:
        return CFG["public_url"].rstrip("/")
    host = request.headers.get("host", f"127.0.0.1:{PORT}") if request else f"127.0.0.1:{PORT}"
    return f"https://{host}"


# ---------------------------------------------------------------- oauth store

STORE_LOCK = threading.Lock()
CODES = {}            # one-time authorization codes (memory only)
FAILED_PINS = []      # timestamps of wrong pairing codes


def _load_store():
    if STORE_FILE.exists():
        try:
            return json.loads(STORE_FILE.read_text(encoding="utf-8"))
        except Exception:
            pass
    return {"clients": {}, "tokens": {}}


STORE = _load_store()


def _save_store():
    now = time.time()
    STORE["tokens"] = {k: v for k, v in STORE["tokens"].items() if v["exp"] > now}
    STORE_FILE.write_text(json.dumps(STORE, indent=1), encoding="utf-8")


def _h(token):
    return hashlib.sha256(token.encode()).hexdigest()


def issue_tokens(client_id):
    access, refresh = secrets.token_urlsafe(32), secrets.token_urlsafe(40)
    now = time.time()
    with STORE_LOCK:
        STORE["tokens"][_h(access)] = {"kind": "access", "client_id": client_id, "exp": now + ACCESS_TTL}
        STORE["tokens"][_h(refresh)] = {"kind": "refresh", "client_id": client_id, "exp": now + REFRESH_TTL}
        _save_store()
    return {"access_token": access, "token_type": "Bearer", "expires_in": ACCESS_TTL,
            "refresh_token": refresh, "scope": "geminiforge"}


def token_valid(token, kind="access"):
    rec = STORE["tokens"].get(_h(token))
    return rec if rec and rec["kind"] == kind and rec["exp"] > time.time() else None


def get_client(client_id):
    if client_id == CFG["client_id"]:
        return {"client_id": client_id, "client_secret": CFG["client_secret"], "redirect_uris": None,
                "client_name": "Gemini", "auth": "client_secret"}
    return STORE["clients"].get(client_id)


def redirect_ok(client, uri):
    p = urlparse(uri or "")
    if p.scheme != "https" or not p.netloc:
        return False
    if client.get("redirect_uris"):
        return uri in client["redirect_uris"]
    return True


# ---------------------------------------------------------------- oauth routes

def resource_metadata(request):
    base = public_url(request)
    return JSONResponse({"resource": f"{base}/mcp", "authorization_servers": [base],
                         "bearer_methods_supported": ["header"], "scopes_supported": ["geminiforge"]})


def server_metadata(request):
    base = public_url(request)
    return JSONResponse({
        "issuer": base,
        "authorization_endpoint": f"{base}/authorize",
        "token_endpoint": f"{base}/token",
        "registration_endpoint": f"{base}/register",
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic", "none"],
        "scopes_supported": ["geminiforge"],
    })


async def register(request):
    try:
        body = await request.json()
    except Exception:
        return JSONResponse({"error": "invalid_client_metadata"}, status_code=400)
    uris = [u for u in body.get("redirect_uris") or [] if urlparse(u).scheme == "https"]
    if not uris:
        return JSONResponse({"error": "invalid_redirect_uri"}, status_code=400)
    method = body.get("token_endpoint_auth_method") or "client_secret_post"
    client = {"client_id": "gf-" + secrets.token_urlsafe(12), "redirect_uris": uris,
              "client_name": str(body.get("client_name") or "MCP client")[:80],
              "auth": "none" if method == "none" else "client_secret"}
    if client["auth"] != "none":
        client["client_secret"] = secrets.token_urlsafe(32)
    with STORE_LOCK:
        STORE["clients"][client["client_id"]] = client
        _save_store()
    log(tr("mcpClientRegistered", name=client["client_name"], hosts=", ".join(urlparse(u).netloc for u in uris)))
    out = {"client_id": client["client_id"], "redirect_uris": uris, "client_name": client["client_name"],
           "token_endpoint_auth_method": method, "grant_types": ["authorization_code", "refresh_token"],
           "response_types": ["code"], "client_id_issued_at": int(time.time())}
    if "client_secret" in client:
        out["client_secret"] = client["client_secret"]
        out["client_secret_expires_at"] = 0
    return JSONResponse(out, status_code=201)


PAGE = """<!doctype html><html lang="{language}"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>{authTitle}</title>
<style>body{{font:16px/1.6 "Segoe UI",system-ui,sans-serif;background:#131314;color:#e3e3e3;
display:flex;justify-content:center;padding:48px 16px;margin:0}}.c{{max-width:420px;width:100%;background:#1e1f20;
border:1px solid #3c4043;border-radius:16px;padding:28px}}h1{{font-size:20px;margin:0 0 8px}}p{{color:#bdc1c6}}
input{{font:22px Consolas,monospace;letter-spacing:4px;width:100%;box-sizing:border-box;padding:10px 12px;
border-radius:10px;border:1px solid #5f6368;background:#131314;color:#fff;margin:8px 0 16px}}
button{{font:inherit;width:100%;padding:10px;border:0;border-radius:10px;background:#1a73e8;color:#fff;cursor:pointer}}
.e{{color:#f28b82}}small{{color:#9aa0a6}}</style></head><body><div class="c">
<h1>⚒ {linkApp}</h1>
<p>{accessRequest}</p>
<p><label for="pairing_code">{pairCode}</label></p>
{error}<form method="post" action="authorize">{hidden}
<input id="pairing_code" name="pairing_code" inputmode="numeric" autocomplete="off" autofocus required>
<button type="submit">{allowLink}</button></form>
<p><small>{redirectTo}</small></p></div></body></html>"""


def _authorize_params(src):
    keys = ("response_type", "client_id", "redirect_uri", "state", "code_challenge",
            "code_challenge_method", "scope", "resource")
    return {k: src.get(k, "") for k in keys}


def _page(params, client, language, messages, error_key=None):
    hidden = "".join(f'<input type="hidden" name="{k}" value="{html.escape(v)}">' for k, v in params.items())
    labels = {k: html.escape(messages[k]) for k in ("authTitle", "linkApp", "pairCode", "allowLink")}
    labels["accessRequest"] = html.escape(messages["accessRequest"].format(client=client.get("client_name") or "Gemini"))
    labels["redirectTo"] = html.escape(messages["redirectTo"].format(dest=urlparse(params["redirect_uri"]).netloc))
    err = f'<p class="e">{html.escape(messages[error_key])}</p>' if error_key else ""
    return HTMLResponse(PAGE.format(language=language, hidden=hidden, error=err, **labels))


async def authorize(request):
    language = current_language()
    messages = messages_for(language)
    src = dict(request.query_params) if request.method == "GET" else dict(await request.form())
    params = _authorize_params(src)
    client = get_client(params["client_id"])
    if not client:
        return PlainTextResponse(messages["unknownClient"], status_code=400)
    if not redirect_ok(client, params["redirect_uri"]):
        return PlainTextResponse(messages["badRedirect"], status_code=400)
    if params["response_type"] != "code":
        return PlainTextResponse(messages["onlyCode"], status_code=400)
    if client["auth"] == "none" and not params["code_challenge"]:
        return PlainTextResponse(messages["needPkce"], status_code=400)
    if request.method == "GET":
        return _page(params, client, language, messages)

    now = time.time()
    FAILED_PINS[:] = [t for t in FAILED_PINS if now - t < 600]
    if len(FAILED_PINS) >= 5:
        return _page(params, client, language, messages, "tooMany")
    if not hmac.compare_digest(str(src.get("pairing_code", "")).strip(), CFG["pairing_code"]):
        FAILED_PINS.append(now)
        log("⚠️ " + tr("mcpWrongCode"))
        return _page(params, client, language, messages, "wrongCode")
    code = secrets.token_urlsafe(32)
    CODES[code] = {**params, "exp": now + 300}
    log("✅ " + tr("mcpLinked", name=client.get("client_name")))
    q = {"code": code, "iss": public_url(request)}
    if params["state"]:
        q["state"] = params["state"]
    sep = "&" if "?" in params["redirect_uri"] else "?"
    return RedirectResponse(params["redirect_uri"] + sep + urlencode(q), status_code=302)


def _client_auth(request, form):
    cid, secret = form.get("client_id"), form.get("client_secret")
    auth = request.headers.get("authorization", "")
    if auth.lower().startswith("basic "):
        try:
            from urllib.parse import unquote
            raw = base64.b64decode(auth[6:]).decode()
            cid, secret = (unquote(x) for x in raw.split(":", 1))
        except Exception:
            return None
    client = get_client(cid or "")
    if not client:
        return None
    if client["auth"] == "none":
        return client
    return client if secret and hmac.compare_digest(secret, client.get("client_secret", "")) else None


async def token(request):
    form = dict(await request.form())
    client = _client_auth(request, form)
    if not client:
        return JSONResponse({"error": "invalid_client"}, status_code=401)
    grant = form.get("grant_type")
    if grant == "authorization_code":
        rec = CODES.pop(form.get("code", ""), None)
        if not rec or rec["exp"] < time.time() or rec["client_id"] != client["client_id"]:
            return JSONResponse({"error": "invalid_grant"}, status_code=400)
        if form.get("redirect_uri") and form["redirect_uri"] != rec["redirect_uri"]:
            return JSONResponse({"error": "invalid_grant"}, status_code=400)
        if rec["code_challenge"]:
            verifier = form.get("code_verifier", "")
            calc = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
            if not hmac.compare_digest(calc, rec["code_challenge"]):
                return JSONResponse({"error": "invalid_grant", "error_description": "PKCE"}, status_code=400)
        return JSONResponse(issue_tokens(client["client_id"]), headers={"Cache-Control": "no-store"})
    if grant == "refresh_token":
        rt = form.get("refresh_token", "")
        rec = token_valid(rt, "refresh")
        if not rec or rec["client_id"] != client["client_id"]:
            return JSONResponse({"error": "invalid_grant"}, status_code=400)
        with STORE_LOCK:
            STORE["tokens"].pop(_h(rt), None)  # rotate
        return JSONResponse(issue_tokens(client["client_id"]), headers={"Cache-Control": "no-store"})
    return JSONResponse({"error": "unsupported_grant_type"}, status_code=400)


def home(request):
    return PlainTextResponse("GeminiForge MCP server. Endpoint: /mcp")


auth_app = Starlette(routes=[
    Route("/", home),
    Route("/.well-known/oauth-protected-resource", resource_metadata),
    Route("/.well-known/oauth-protected-resource/mcp", resource_metadata),
    Route("/.well-known/oauth-authorization-server", server_metadata),
    Route("/.well-known/oauth-authorization-server/mcp", server_metadata),
    Route("/.well-known/openid-configuration", server_metadata),
    Route("/register", register, methods=["POST"]),
    Route("/authorize", authorize, methods=["GET", "POST"]),
    Route("/token", token, methods=["POST"]),
])


# ---------------------------------------------------------------- approval

APPROVAL_LOCK = threading.Lock()
ALLOWED_UNTIL = {}


def approve(tool, args):
    mode = CFG.get("approval", "readonly")
    if mode == "none" or (mode == "readonly" and tool in READ_ONLY):
        return True
    if ALLOWED_UNTIL.get(tool, 0) > time.time():
        return True
    pyw = Path(sys.executable).with_name("pythonw.exe")
    exe = str(pyw if pyw.exists() else sys.executable)
    # Gemini gives up on a tool call after about a minute, so the popup auto-denies at 45 s.
    with APPROVAL_LOCK:  # one popup at a time
        try:
            r = subprocess.run([exe, str(HERE / "approve_dialog.py"), tool,
                                json.dumps(args, ensure_ascii=False), "45"], timeout=60)
            code = r.returncode
        except subprocess.TimeoutExpired:
            code = 1
    if code == 2:
        ALLOWED_UNTIL[tool] = time.time() + 1800
    return code in (0, 2)


# ---------------------------------------------------------------- tools

DESCRIPTIONS = {
    "cd": "Change the current working folder (later relative paths resolve against it).",
    "list_dir": ("List a folder; use it to locate projects. directories_only=true lists subfolders only; "
                 "depth defaults to 1, max 4. Drive roots work as C:/ or D:/."),
    "read_file": "Read a text file with line numbers. Read large files in parts with offset/limit.",
    "write_file": "Create or overwrite a whole file (UTF-8). Use edit_file for small changes. The original is backed up first.",
    "edit_file": ("Replace text in a file that exactly matches old (including spaces and indentation) with new. "
                  "Check the content with read_file first. The original is backed up first."),
    "list_backups": "List backups made by write_file / edit_file (newest first); path limits it to a file or folder.",
    "restore_backup": ("Restore a file to a backup (backup_id from list_backups). If the file did not exist before, "
                       "restoring deletes it. The current content is backed up first, so a restore can be undone."),
    "search": "Search file contents with a regular expression; returns file:line: text.",
    "find_files": "Find files by filename wildcard, e.g. *.py.",
    "run": ("Run a command in Windows PowerShell and return its output. Use the dedicated tools to list folders, "
            "find files, search or read files instead of run. timeout is the total limit (default 120 s, max 900). "
            "If it is still running after 40 s, it replies early with a job id and keeps running in the background; "
            "get the result with job_status."),
    "job_status": ("Check a background job from run: with id, returns its status and output, waiting up to seconds "
                   "(max 40) for it to finish; without id, lists all jobs."),
    "stop_job": "Stop a background job from run (including the programs it started).",
    "start_process": "Start a long-running program in the background (e.g. a dev server); output goes to a log file.",
    "open": "Open a file, folder, URL or app with its default program (e.g. calc, notepad).",
    "screenshot": "Capture the screen (the monitor with the mouse by default, or a given monitor). Purple grid lines every 100px.",
    "zoom": "Zoom into part of the last screenshot (for small text). Does not change click coordinates.",
    "click": "Click with the real mouse at (x, y) of the last screenshot. Prefer click_element when possible.",
    "double_click": "Double-click at screenshot coordinates (x, y).",
    "right_click": "Right-click at screenshot coordinates (x, y).",
    "move": "Move the mouse to screenshot coordinates (x, y).",
    "drag": "Drag from screenshot coordinates (x1, y1) to (x2, y2).",
    "type": "Type text at the current keyboard focus (any language). enter=true presses Enter at the end.",
    "key": "Press a key or combination, e.g. enter, ctrl+c, alt+tab, win+r. Shortcuts act on the front window.",
    "scroll": "Scroll the mouse wheel: positive = up, negative = down; optional screenshot coordinates x, y.",
    "wait": "Wait a number of seconds (max 30).",
    "clipboard_get": "Read text from the clipboard.",
    "clipboard_set": "Put text on the clipboard.",
    "list_windows": "List all open windows (including minimized ones).",
    "focus_window": ("Switch to a window exactly by window_id, or by title (exact match preferred). "
                     "Stops if the window cannot be focused."),
    "ui_elements": ("List clickable buttons, links and input boxes in a window (the active window by default, or "
                    "pick one by title with window). Returns window_id, element ids and names. Pass the same "
                    "window_id to later calls; include_text=true also lists text."),
    "click_element": ("Click an element listed by ui_elements. Prefer name (copied exactly from the list) over the id. "
                      "Buttons are triggered directly (no mouse movement, works with background windows), and the "
                      "window's current text is returned."),
    "click_elements": ("Click 1-16 buttons from names in order, all in the window given by window_id. Names must come "
                       "from the latest ui_elements. Review the whole sequence first; it stops at the first failure and "
                       "reports how many steps finished. Example for Calculator: "
                       "names=[\"Clear\",\"One\",\"Two\",\"Multiply by\",\"Three\",\"Equals\"]."),
}
TYPES = {
    "path": str, "content": str, "old": str, "new": str, "pattern": str, "glob": str, "command": str,
    "cwd": str, "target": str, "text": str, "keys": str, "title": str, "window": str, "filter": str,
    "name": str, "button": str, "backup_id": str,
    "offset": int, "limit": int, "depth": int, "timeout": int, "x": int, "y": int, "x1": int, "y1": int,
    "x2": int, "y2": int, "clicks": int, "amount": int, "monitor": int, "max": int, "id": int,
    "seconds": float, "window_id": int, "names": list[str],
    "replace_all": bool, "ignore_case": bool, "grid": bool, "enter": bool, "include_text": bool, "directories_only": bool,
}

INSTRUCTIONS = """GeminiForge lets you operate the user's Windows PC: read and write files, run PowerShell, write code, see the screen and control the mouse and keyboard.
Use ~ for the user's home folder; forward slashes work in paths. Do not guess the user name or the real home path; look it up when needed.
Reply to the user in the language they write in.
- To locate a project, start with list_dir(path, directories_only=true); to check several known places, call it once per place. Use find_files for file names, search for text and read_file to read files; do not call PowerShell for these.
- read_file before changing code; use edit_file for small changes. write_file / edit_file back up the original; if something breaks, find the id with list_backups and restore with restore_backup. Files changed by commands from run are not backed up.
- To click a button or link: find it with ui_elements (optionally window, filter), then click it with click_element {name}; this is more accurate than estimating screenshot coordinates.
- click_element returns the window's current text, which you can use to check the result.
- The window_id from ui_elements is the target window; pass the same window_id to later clicks so a browser tab with the same name is not hit instead.
- For a run of observed buttons with predictable results, prefer click_elements(window_id, names) so the whole sequence is reviewed once.
- If a tool reports an error, stop that step and observe again; never treat a tool error as done and never blindly click old coordinates.
- focus_window the target program before click/type/key. Close programs with their own close button, not alt+f4.
- Several tool calls at once are fine when the results are predictable.
- run returns a job id after 40 s and keeps running in the background; poll job_status {id, seconds: 30} until it finishes instead of rerunning the command.
- Ask the user before risky actions such as deleting files, installing software, paying or submitting forms.
- If a tool is denied, stop; do not use other tools to get around the denial. Ask the user first."""


def run_tool(name, kwargs):
    args = {k: v for k, v in kwargs.items() if v is not None}
    log(f"{name} {json.dumps(args, ensure_ascii=False)[:200]}")
    if not approve(name, args):
        log("   ⛔ " + tr("mcpDenied", tool=name))
        raise MCPToolError("The user denied this action on the computer. Stop; do not use other tools to get around the denial.")
    try:
        text, image = gf.call_tool(name, args)
    except gf.ToolError as e:
        raise MCPToolError(str(e)) from e
    except gf.pyautogui.FailSafeException:
        raise MCPToolError("The user moved the mouse to a screen corner; emergency stop.") from None
    except TypeError as e:
        raise MCPToolError(f"Invalid arguments: {e}") from e
    except Exception as e:
        raise MCPToolError(f"Error: {type(e).__name__}: {e}") from e
    if image:
        return [text, Image(data=base64.b64decode(image), format="jpeg")]
    return text


def make_tool(name, fn):
    params = []
    for p in inspect.signature(fn).parameters.values():
        t = TYPES.get(p.name, str)
        if p.default is None:
            t = Optional[t]
        params.append(p.replace(annotation=t, kind=inspect.Parameter.KEYWORD_ONLY))
    if name in gf.SCREEN_ACTIONS:
        params.append(inspect.Parameter("screenshot", inspect.Parameter.KEYWORD_ONLY, default=True, annotation=bool))

    async def tool(**kwargs):
        return await anyio.to_thread.run_sync(run_tool, name, kwargs)

    tool.__name__ = name
    tool.__doc__ = DESCRIPTIONS.get(name, name)
    tool.__signature__ = inspect.Signature(params)
    tool.__annotations__ = {p.name: p.annotation for p in params}
    return tool


def simplify_schema(schema):
    """Plain JSON schema that strict clients (Gemini) accept: no anyOf/null, no titles."""
    props = {}
    for key, p in schema.get("properties", {}).items():
        p = dict(p)
        if "anyOf" in p:
            types = [x for x in p.pop("anyOf") if x.get("type") != "null"]
            if types:
                p.update(types[0])
        if p.get("default", 0) is None:
            p.pop("default")
        p.pop("title", None)
        props[key] = p
    out = {"type": "object", "properties": props}
    if schema.get("required"):
        out["required"] = list(schema["required"])
    return out


mcp = MCPServer(name="GeminiForge", title="GeminiForge", version="1.0.0", instructions=INSTRUCTIONS)
for _name, _fn in gf.TOOLS.items():
    ro = _name in READ_ONLY or CFG.get("web_confirm", "on") == "off"
    mcp.add_tool(make_tool(_name, _fn), name=_name, description=DESCRIPTIONS.get(_name, _name),
                 annotations=ToolAnnotations(read_only_hint=ro, destructive_hint=not ro),
                 structured_output=False)
for _tool in mcp._tool_manager.list_tools():
    _tool.parameters = simplify_schema(_tool.parameters)

mcp_app = mcp.streamable_http_app(
    # Requests arrive through Tailscale with the public host name; access is
    # controlled by the OAuth bearer token below instead.
    transport_security=TransportSecuritySettings(enable_dns_rebinding_protection=False),
    # Stateless: no session IDs, so Gemini keeps working (and re-reads tools) after a restart.
    stateless_http=True,
)


# ---------------------------------------------------------------- asgi entry

async def _unauthorized(scope, receive, send):
    meta = f"{CFG['public_url'] or ''}/.well-known/oauth-protected-resource"
    body = json.dumps({"error": "invalid_token"}).encode()
    await send({"type": "http.response.start", "status": 401, "headers": [
        (b"content-type", b"application/json"),
        (b"www-authenticate", f'Bearer resource_metadata="{meta}", scope="geminiforge"'.encode()),
        (b"content-length", str(len(body)).encode())]})
    await send({"type": "http.response.body", "body": body})


ACCESS_LOG = HERE / "mcp_access.log"


def _access_log(scope, status):
    h = dict(scope.get("headers") or [])
    ua = h.get(b"user-agent", b"").decode(errors="replace")[:60]
    fwd = h.get(b"x-forwarded-for", b"").decode(errors="replace")
    line = (f"{time.strftime('%m-%d %H:%M:%S')} {scope.get('method')} {scope['path']}"
            f"{'?' + scope['query_string'].decode()[:120] if scope.get('query_string') else ''}"
            f" -> {status} [{fwd}] {ua}")
    try:
        with open(ACCESS_LOG, "a", encoding="utf-8") as f:
            f.write(line + "\n")
    except OSError:
        pass
    if scope["path"] != "/mcp" or status != 200:
        print("  " + line, flush=True)


async def app(scope, receive, send):
    if scope["type"] != "http":
        return await mcp_app(scope, receive, send)  # lifespan
    status = {"code": 0}
    is_mcp = scope["path"] == "/mcp"
    out_chunks = []

    async def send_logged(msg):
        if msg["type"] == "http.response.start":
            status["code"] = msg["status"]
        elif msg["type"] == "http.response.body" and is_mcp and sum(map(len, out_chunks)) < 4000:
            out_chunks.append(msg.get("body", b""))
        await send(msg)

    # Note which JSON-RPC methods Gemini calls (body is replayed to the app).
    if is_mcp and scope.get("method") == "POST":
        body = b""
        while True:
            m = await receive()
            body += m.get("body", b"")
            if not m.get("more_body"):
                break
        _rpc_log("→", body)
        sent = {"done": False}

        async def receive_replay():
            if not sent["done"]:
                sent["done"] = True
                return {"type": "http.request", "body": body, "more_body": False}
            return await receive()
        rcv = receive_replay
    else:
        rcv = receive

    try:
        await _route(scope, rcv, send_logged)
    finally:
        _access_log(scope, status["code"])
        if out_chunks:
            _rpc_log("←", b"".join(out_chunks))


def _rpc_log(direction, raw):
    text = raw.decode("utf-8", errors="replace")
    try:
        msgs = []
        for chunk in [l[5:] for l in text.splitlines() if l.startswith("data:")] or [text]:
            obj = json.loads(chunk)
            for o in obj if isinstance(obj, list) else [obj]:
                if "method" in o:
                    name = o.get("params", {}).get("name", "")
                    msgs.append(f"{o['method']}{' ' + name if name else ''}")
                elif "error" in o:
                    msgs.append(f"ERROR {json.dumps(o['error'], ensure_ascii=False)[:300]}")
                elif "result" in o:
                    r = o["result"]
                    if isinstance(r, dict) and r.get("isError"):
                        msgs.append(f"tool error {json.dumps(r.get('content'), ensure_ascii=False)[:200]}")
                    else:
                        msgs.append("result " + ",".join(list(r)[:4]) if isinstance(r, dict) else "result")
        line = f"{time.strftime('%m-%d %H:%M:%S')}   {direction} " + " | ".join(msgs)
    except Exception:
        line = f"{time.strftime('%m-%d %H:%M:%S')}   {direction} (raw) {text[:200]}"
    try:
        with open(ACCESS_LOG, "a", encoding="utf-8") as f:
            f.write(line + "\n")
    except OSError:
        pass


async def _route(scope, receive, send):
    if scope["type"] == "http":
        path = scope["path"]
        if path == "/mcp" or path.startswith("/mcp/"):
            auth = dict(scope["headers"]).get(b"authorization", b"").decode()
            if not (auth[:7].lower() == "bearer " and token_valid(auth[7:].strip())):
                return await _unauthorized(scope, receive, send)
            return await mcp_app(scope, receive, send)
        return await auth_app(scope, receive, send)
    return await mcp_app(scope, receive, send)  # lifespan


def main():
    print("=" * 60)
    print(" " + tr("mcpTitle"))
    print("=" * 60)
    print(" " + tr("mcpUrl", url=(CFG["public_url"] or tr("mcpNoTailscale")) + "/mcp"))
    print(" " + tr("mcpPairCode", code=CFG["pairing_code"]))
    print(" " + tr("mcpApproval", mode=CFG["approval"]))
    print(" " + tr("mcpWebConfirm", mode=CFG.get("web_confirm", "on")))
    print(" " + tr("mcpStop"))
    print("=" * 60, flush=True)
    uvicorn.run(app, host="127.0.0.1", port=PORT, log_level="warning")


if __name__ == "__main__":
    main()
