# GeminiForge local server

The GeminiForge server program, written in Rust. One executable, `geminiforge-server.exe` (about 4 MB), holds the tool server for the Chrome extension (`127.0.0.1:18765`: `GET /health`, `POST /tool`, `POST /language`), the MCP entry for Gemini's "@ custom app", backup and restore, and the approval window.

## Build and start

Needs the Rust toolchain (`cargo`, MSVC target, from https://rustup.rs).

```bat
cd server
cargo build --release
```

This produces `server\target\release\geminiforge-server.exe`. You rarely need to build by hand: the batch files in the project folder find the program through `server\exe.cmd` and build it once if it is missing. `exe.cmd` looks for `server\target\release\geminiforge-server.exe`, then `server\geminiforge-server.exe` (so a prebuilt exe can simply be dropped there).

The same executable has four uses:

| Command | Batch file | Purpose |
|---|---|---|
| `geminiforge-server.exe` | `start.bat` | Tool server for the extension (18765) |
| `geminiforge-server.exe mcp` | `start_mcp.bat` | MCP entry for Gemini's "@ custom app" (18767) |
| `geminiforge-server.exe backups [list [N] \| restore ID]` | `restore.bat` | List and restore backups |
| `geminiforge-server.exe approve TOOL ARGS SECONDS` | (called by the MCP entry) | Desktop approval window |

Start with Windows: `autostart_on.bat` / `autostart_off.bat`.

### macOS and Linux

The same code builds on macOS and Linux (`cargo build --release` gives `server/target/release/geminiforge-server`); the `.sh` scripts next to the batch files (`start.sh`, `start_mcp.sh`, `restore.sh`, `autostart_on.sh`, `autostart_off.sh`) do what the batch files do, through `server/exe.sh` and `server/lang.sh`. What differs:

- Only the file and command tools (`cd` … `restore_backup`, `run`, `job_status`, `stop_job`, `start_process`, `open`, `wait`, clipboard) and `screenshot` are there. The desktop tools listed in `tools::DESKTOP_TOOLS` answer that they are not available yet, and the MCP entry does not list them. `GET /health` adds `"os": "macos"` or `"linux"`, which makes the extension use `extension/prompts/unix/<language>.md`; the MCP entry sends `src/mcp_instructions_unix.txt`.
- `run` and `start_process` use bash (sh when there is no bash); every command leads its own process group, which timeouts and `stop_job` kill. `open` uses `open` (macOS) or `xdg-open` (Linux), or starts a program by name.
- The approval window is an AppleScript dialog on macOS and zenity or kdialog on Linux; without either (or without a desktop session) actions that need approval are denied, and the MCP window says so at start.
- `screenshot` runs the system's capture program for a PNG (`screencapture` on macOS, which needs Screen Recording permission; on Linux the first of `grim`, `gnome-screenshot`, `spectacle`, `scrot`, ImageMagick's `import`), decodes it with the `png` crate and sizes it down like on Windows. Window capture, the grid and `monitor` on Linux are not supported.
- The clipboard goes through `pbcopy`/`pbpaste`, `wl-copy`/`wl-paste`, `xclip` or `xsel`. Text that is not UTF-8 is read with replacement characters (no cp950/gbk fallback). The language preference is `~/.config/GeminiForge/language.txt`.
- Platform code lives in `src/unix/`: `ffi.rs` (time, random bytes, killing a process group), `popup.rs` (approval dialog), `gui_tools.rs` (screenshot, clipboard) and stand-ins for the desktop tools. `.github/workflows/build.yml` builds and tests Windows, macOS (arm64, x64) and Linux (x64, arm64, static musl) and packs a zip per system.

Environment variables:

| Variable | Purpose |
|---|---|
| `GF_PORT` | Port, default 18765 (change the extension too if you change it) |
| `GF_ROOT` | The GeminiForge folder. By default the first folder above the exe that contains `extension\_locales` |
| `GF_BACKUPS` | Backup folder. Default `<GF_ROOT>\backups` |
| `GF_FAST_WAIT` | `1` turns on fast waits (see below). Off when the exe is run directly; `start.bat` and `start_mcp.bat` set it to `1` |
| `GF_MCP_PORT` | Port of the MCP entry, default 18767 |
| `GF_MCP_DIR` | Folder of `mcp_config.json`, `mcp_auth.json` and `mcp_access.log`. Default `<GF_ROOT>\server` |

## MCP entry (`geminiforge-server.exe mcp`)

OAuth (dynamic registration, pairing code, PKCE, token rotation), the MCP protocol, the desktop approval window and the access log.

- Pairing data is kept in `server\mcp_config.json` and `mcp_auth.json`.
- Restart after changing the `approval`, `web_confirm` or `tunnel` settings.
- `"tunnel": "cloudflared"` (or `GF_TUNNEL=cloudflared`) makes the entry start `cloudflared tunnel --url http://127.0.0.1:<port>` itself and use the quick tunnel's `https://….trycloudflare.com` address for this run instead of `public_url`. That address changes at every start, so the custom app in Gemini has to be updated and paired again each time. cloudflared is not downloaded for you; it is looked up on PATH and where its installers put it.
- The tool list (names, descriptions, parameter schemas) and the instructions for the model are built into the executable from `src/mcp_tools.json` and `src/mcp_instructions.txt`. When a tool's description or parameters change, update both files.
- The approval window is a native Windows window in the system's default light style.
- The `GET /mcp` event stream stays open and sends keep-alive comments; at most 32 streams at a time.

## Fast waits (`GF_FAST_WAIT=1`)

Without it, many steps wait a fixed time. With it, each step goes on as soon as what it waits for has happened; the fixed time is only the upper limit, so it is never slower than fixed waits.

| Where | Fixed wait | Fast wait |
|---|---|---|
| Bringing a window to the front | 0.5 s | No switch and no wait if the window is already in front; otherwise go on once it is in front (plus 0.05 s). Restoring a minimized window still waits 0.5 s |
| After `click_element` / `click_elements` | 0.5 + 0.4 s | Re-read the window's controls; go on once they changed and two reads in a row match. Wait the full time only if nothing changed |
| `scroll` with `percent`, scrolling a list item into view | 0.5 + 0.2 s | Go on once the position is reached and stops moving |
| After a screen action, before the screenshot | 0.8 s | Note the screen before the action; take the screenshot once it changed and stayed still for 0.2 s. Wait the full time only if it did not change or keeps changing (a playing video) |
| After `open` | 1.5 s | Go on once a new window appears or the foreground window changes |

Unchanged: the gaps between mouse and keyboard events, the short waits around pasting, the timing of clicks that do not take focus (`background` / `cursor_jump`), and the 40 seconds of `run`.

Measured on one machine with the same test window:

| Action | Fixed wait | Fast wait |
|---|---|---|
| `click_element` (no screenshot) | 1.3–1.5 s | 0.6–1.0 s |
| `click_elements`, 4 clicks | 4.8 s | 2.2 s |
| `scroll` to a percentage | 1.5 s | 0.3 s |
| `click` with `window_id` plus screenshot | 1.6 s | 0.75 s |
| `open` a file | 1.7 s | 0.6 s |

Things to know:

- When the window is already in front, no Alt key press is sent first. This also avoids a problem where Alt puts the window into menu mode and swallows the next typed character.
- A screen that reacts in two steps (a quick flash, then content more than 0.2 s later, such as a slow web page) may be captured before the content arrives. Take another screenshot in the next step, or use fixed waits.
- Each screen action spends a few tens of milliseconds noting the screen first.
- In this mode `/health` has an extra `"fastWait": true` field, so you can tell which mode is running.

## Tests

```bat
cargo test
```

Covers files and backups, commands and background jobs, the HTTP safety checks, window selection, the UI tools' safeguards, screenshots and OCR. `tests/mcp.rs` covers the MCP entry: registration and the client limit, lockout after five wrong pairing codes, PKCE, token rotation, checking the token before reading the body, one approval window at a time, log rotation, the tool schemas, and the read-only tool list in `extension/content.js`. The tests never click or type and never show the approval window; they only read one visible window, take screenshots and run OCR on generated images.

## Other behavior

- **`search` regular expressions** use `fancy-regex`, with lookaround and backreferences.
- **JSON parsing is strict**: no `NaN` / `Infinity` or lone surrogates, nesting up to 128 levels.
- **Files in legacy encodings**: text that is not UTF-8 is tried as cp950, then gbk (Windows code pages, private-use characters excluded).
- **OCR** uses Windows.Media.Ocr in-process.
- **Typing**: ASCII characters the current keyboard layout cannot type are skipped.
- **Relative `open` targets** resolve against the server's working folder (`server\`).
- **Links in `find_files` / `search`**: junctions are followed, directory symlinks are not.
- Message texts are read from `extension\_locales` at run time; if the project folder cannot be found, the copy built into the exe is used.

## Paths not tested on a real machine

- `ui_elements` / `read_window` / `ocr_window` without a window, reading the current foreground window (avoided so the tests do not read the window in use; the code path is the same as with a window).
- Restoring a minimized or hidden window automatically.
- The screen-crop fallback when `PrintWindow` fails (DirectX, protected windows).
- The slow walk when reading the whole UIA tree from the cache fails; a node that fails is skipped.
- OCR timeouts (15 s) and missing language packs.
- The MCP entry has not been tested over Tailscale with the real Gemini, only with a scripted client on this machine. The approval window was checked for layout and automatic denial on timeout; the buttons themselves are verified by tests with a stand-in.

## Code layout

| File | Contents |
|---|---|
| `http.rs`, `main.rs` | HTTP server and entry point |
| `tools.rs` | Tool table, argument checks, global lock, UI thread, screenshot after actions |
| `py.rs` | Shared rules for number and text conversion, slicing and JSON output |
| `fs_tools.rs`, `backups.rs` | File tools and backups |
| `proc_tools.rs` | `run`, background jobs, `open`, `wait` |
| `input.rs` | Mouse and keyboard |
| `win.rs`, `image.rs`, `gui_tools.rs` | Windows, clipboard, screenshots, drawing, click tools |
| `uia.rs`, `ui_tools.rs` | UI Automation and its eight tools |
| `ocr.rs`, `ocr_tools.rs` | Windows OCR, `ocr_window`, `click_text` |
| `mcp.rs`, `approve.rs`, `approve_win32.rs`, `crypto.rs` | MCP entry, approval window, SHA-256 and tokens |
| `lang.rs`, `state.rs`, `ffi.rs` | Language, shared state, Win32 declarations |
| `unix/` | macOS and Linux versions of `ffi.rs` and the approval window, the clipboard, stand-ins for the desktop tools |
