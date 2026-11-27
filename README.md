**English** | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md)

# GeminiForge

Lets the Gemini web app (gemini.google.com) operate your computer: read and write files, run commands, write code, see the screen, and control the mouse and keyboard.
It uses your Gemini web quota, so no API key is needed.

```
gemini.google.com  ──(Gemini outputs tool commands)──▶  Chrome extension  ──▶  local server 127.0.0.1:18765
        ▲                                                              │ files / PowerShell / screenshots / mouse and keyboard
        └──────(results and screenshots are pasted back and sent)◀─────┘
```

## Download

Download `GeminiForge-<version>-windows-x64.zip` (Windows) from [Releases](../../releases/latest) and extract it to any folder. It includes the prebuilt server (`server\geminiforge-server.exe`), so nothing else needs to be installed. If Windows shows a SmartScreen warning the first time, choose "More info" → "Run anyway".

To build from source instead, install [Rust](https://rustup.rs); `start.bat` builds the server on its first run (about a minute).

For a Mac or Linux computer, see the next section.

## macOS and Linux

The Chrome extension (Option 1), MCP (Option 2), and the file and command tools work on macOS and Linux too. Gemini can look at the screen with `screenshot`, but controlling the mouse and keyboard is Windows-only for now; GeminiForge tells Gemini so on its own.

1. Download the zip for your computer from [Releases](../../releases/latest): `macos-arm64` (Apple silicon), `macos-x64` (Intel Mac), `linux-x64` or `linux-arm64`, and extract it.
2. Instead of the `.bat` files, run the `.sh` scripts in a terminal:
   ```
   cd GeminiForge
   sh start.sh            # Option 1: server for the extension
   sh start_mcp.sh        # Option 2: MCP server
   sh restore.sh          # restore a backup
   sh autostart_on.sh     # start at login (autostart_off.sh turns it off)
   ```
   Keep the terminal open while you use GeminiForge. Everything else in Option 1 and Option 2 is the same.

- `run` uses bash instead of PowerShell.
- **macOS**: the first start may say the program cannot be opened because the developer cannot be verified. Open System Settings → Privacy & Security, click "Allow Anyway" for geminiforge-server, and start again.
- **Linux**: the MCP approval window needs `zenity` (or `kdialog`), and the clipboard tools need `wl-clipboard` (Wayland) or `xclip` (X11), for example `sudo apt install zenity wl-clipboard xclip`.
- **Screenshots**: on macOS, allow Screen Recording for the terminal app that starts GeminiForge (System Settings → Privacy & Security → Screen Recording). On Linux, one of `gnome-screenshot`, `grim`, `spectacle`, `scrot` or ImageMagick has to be installed.
- The Tailscale tunnel for Option 2: `tailscale funnel --bg --https=10000 http://127.0.0.1:18767` (with the Mac App Store app, the command is `/Applications/Tailscale.app/Contents/MacOS/Tailscale`).
- To build from source, install [Rust](https://rustup.rs); `start.sh` builds the server on its first run.

## Option 1: Chrome extension

### First-time setup

1. **Start the local server**: double-click `start.bat`. Keep the black window open; closing it stops the server.
2. **Install the Chrome extension**:
   1. Enter `chrome://extensions` in Chrome's address bar
   2. Turn on "Developer mode" in the top-right corner
   3. Click "Load unpacked" and choose the `extension` folder inside this folder
3. **Open Gemini**: go to gemini.google.com (refresh the tab if it is already open). The **⚒ GeminiForge** panel appears in the bottom-right corner; a green dot means it is connected to the server.
4. **First screenshot upload**: click "Test screenshot" in the panel. If Gemini shows a consent dialog about creating content from images and files, accept it (only needed once). After the test screenshot lands in the message field, click the ✕ on the attachment to remove it.

### How to use

Open a **new chat** in Gemini, type what you want and send it, for example "Create hello.py on my desktop and run it".
The extension adds the tool instructions in front of your first message automatically (when "Add tool instructions to new chats" is checked in the panel).

From then on the extension picks up the tool commands in Gemini's replies, runs them according to the execution mode (or waits for you to click "Run"), and sends the results back to Gemini, repeating until the task is done.

To use it in a chat that has already started, click "Insert tool instructions" in the panel.

The execution mode in the bottom-right corner only controls the extension's `gf_tool` JSON commands. If "Let Gemini use … from GeminiForge" with Allow / Deny appears in the middle of the page, that turn is using the MCP connector; the extension's "Run all automatically" does not remove that confirmation. To use extension mode, start a new chat, choose Chat mode, do not mention `@GeminiForge`, and keep "Add tool instructions to new chats" checked.

**A Gem works too**: create a new Gem from the Gems page in Gemini's sidebar and paste the contents of `extension/prompts/en.md` (or the `extension/prompts/<language>.md` for another language) into its instructions. In chats inside that Gem the extension will not add the instructions again.

**Use Pro or a thinking model**: for multi-step tasks (writing code, operating the computer) Pro is far more reliable than Flash.

## Option 2: @ custom app (MCP, recommended)

This is Gemini's official way to connect tools: type `@GeminiForge` in Gemini's message field. It needs no Chrome extension, does not break when the Gemini page changes, and also uses your web quota.

```
gemini.google.com ──(MCP, OAuth)──▶ https://<your-pc>.ts.net:10000 ──Tailscale Funnel──▶ 127.0.0.1:18767 (geminiforge-server.exe mcp)
```

### First-time setup

1. **Start the MCP server**: double-click `start_mcp.bat`. The window shows the **MCP URL** and the **pairing code**; keep it open.
2. **Open the public tunnel** (only once): run in PowerShell
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --bg --https=10000 http://127.0.0.1:18767
   ```
   Funnel prints this computer's public URL; add `/mcp` to it to get the connector URL, for example `https://<device-name>.<tailnet>.ts.net:10000/mcp`. Use your own URL. To close the tunnel:
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --https=10000 off
   ```
   No Tailscale? Install [cloudflared](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/) and set `"tunnel": "cloudflared"` in `server/mcp_config.json`: the MCP server then opens a quick tunnel by itself and shows its URL. That URL changes every time the server starts, so after each start put the new URL into the custom app in Gemini and pair again.
3. **Add it in Gemini**: type `@` in Gemini's message field → view all connectors → create a custom app → paste the MCP URL → continue.
   - The client ID / secret under the additional settings can be left empty at first. If Gemini asks for them, use the `client_id` and `client_secret` from `server/mcp_config.json`.
4. When the **Connect to GeminiForge** page opens, enter the **pairing code** from the server window and click "Allow connection".

### Using it

1. Open Gemini, either way:
   - **Gemini desktop app** (Windows): works in Chat mode even with a non-English UI; GeminiForge is right there in the `@` menu, with no need to switch to the English UI or Task mode (tested on 2026-11-23 with app 1.21.1). The desktop app can only use this option; the Chrome extension from Option 1 cannot be loaded into it.
   - **Web**: use the **English UI** at `https://gemini.google.com/app?hl=en`. With the English UI, @GeminiForge works in ordinary Chat mode, and Gemini even calls it without the @.
     With a non-English UI, custom apps do not appear in the @ menu in Chat mode; you have to switch to **Task** mode (tested 2026-11).
2. Type `@`, pick **GeminiForge** from the menu, then type your request, for example "List the files on my desktop".

For actions that change something, **the Gemini page asks first** with "Let Gemini use … from GeminiForge"; the action only runs after you click Allow.

The server then shows a **second approval window** on your computer (when `approval` is `readonly`, the default):
- **Allow**: this time only
- **Allow this tool for 30 minutes**: the same kind of action is not asked again
- **Deny**, or no answer for 45 seconds, which denies automatically (Gemini gives up after about a minute)

The approval mode can be changed in `approval` in `server/mcp_config.json`, followed by a server restart: `readonly` (default: Gemini web confirmation + approval window on the computer), `all` (the window also appears for read-only actions), `none` (only the Gemini web confirmation, no window on the computer).

### Security

- The pairing code is required to connect; 5 wrong attempts lock it for 10 minutes.
- Do not share `server/mcp_config.json` (pairing code, secret) or `server/mcp_auth.json` (login credentials) with anyone.
- To disconnect every connected device: stop the server, delete `server/mcp_auth.json`, and start it again. To change the pairing code: delete the `pairing_code` line in `mcp_config.json` and restart.
- Once the tunnel is open anyone can reach the URL, but no tool can be used without the pairing code.

## The panel

Expand **⚒ GeminiForge** in the bottom-right corner of the Gemini page and use the "Language" menu to switch between **English, 繁體中文, 简体中文, 日本語, 한국어**. The first time, it follows Chrome's UI language; any other language falls back to English. The choice is saved in the current Chrome profile and synced to the Gemini tabs that are already open.

The panel, the confirmation buttons, and tool instructions inserted afterwards all use the selected language. Once the local server is connected, the MCP pairing page and the desktop approval window also use it the next time they open. When updating an existing install, reload the extension, refresh the Gemini tab, and restart the local server started by `start.bat`. If you also use MCP, restart `start_mcp.bat` as well.

| Item | What it does |
|---|---|
| Dot | Green: server connected; red: run `start.bat` |
| Enable | When off, the extension runs no tools |
| Add tool instructions to new chats | Puts the tool instructions in front of the first message you send in a new chat |
| Notify me when Gemini is done | Shows a Windows notification when Gemini finishes replying (or the whole tool task finishes), when it is waiting for you to click "Run", or when something goes wrong; clicking the notification switches back to that Gemini tab. The default "Only when away" notifies only while you are on another tab or app, not while you are looking at the Gemini tab. "Always" notifies even while you are looking at the tab, and "Off" never notifies. "Test notification" next to it sends one right away so you can check whether Windows is blocking it. Each notification adds a 🔔 line to the panel log. "Enable" must be checked too |
| Execution mode | **Ask at every step**: safest<br>**Ask except for read-only tools** (default): reading files and taking screenshots run automatically; writing files, running commands, and clicking need you to click "Run"<br>**Run all automatically**: fully hands-off; use it only for tasks you trust |
| Stop | Aborts the current run immediately |
| "Allow for this chat" in the confirmation box | This tool is not asked about again in the same chat (resets in a new chat) |
| Test screenshot | Checks that screenshots and uploads work |

After 200 automatic rounds in a row it stops and asks whether to continue.

If no notification appears: in Windows "Settings → System → Notifications", make sure notifications for **Google Chrome** are on and "Do not disturb" is off. If you just updated the extension, click reload (↻) on GeminiForge in `chrome://extensions` so the extension gets the notification permission. After the Gemini tab has been in the background for more than 5 minutes, Chrome checks it less often, so a notification can be about a minute late.

## Start with Windows (optional)

- Double-click `autostart_on.bat`: the server then starts in a minimized window at every boot
- Double-click `autostart_off.bat`: turns it off

## Tools Gemini can use

- **Files and programs**: cd, list_dir, read_file, read_files, write_file, edit_file, edit_files, search, find_files, run (PowerShell), job_status, stop_job, start_process, open, clipboard
- **Backup and restore**: list_backups, restore_backup (see "File backups" below)
- **Operating the computer**: screenshot, zoom (magnify small text), click, double_click, right_click, move, drag, type, key, scroll (supports targeting by `window_id`, jumping by `percent`, and automatic UIA scrolling of inner containers as a fallback)
- **Precise clicking and reading**: list_windows, focus_window, ui_elements (lists buttons / links plus the values and checked state of inputs), read_window (reads the text and field values of a window's or web page's main content in one call, including what is below the scroll position), click_element (clicks by name or number, scrolls off-screen elements into view, and reports the window text), type_element (focuses an input, clears it, types, and reads the value back in one call)

`ui_elements` returns a `window_id`; passing the same ID to later actions avoids picking a browser tab with the same name. A series of buttons that has already been observed can be run in order with `click_elements {window_id, names}` (up to 16); it stops at the first failure. In MCP mode the Gemini page still confirms the whole series; this does not remove the web confirmation. When a window or element is no longer valid, the tool reports an error and asks for a fresh observation instead of clicking old coordinates.

The English tool instructions are in `extension/prompts/en.md`, the other languages are in `extension/prompts/`, and you can edit them.

## File backups

Every time Gemini changes a file with `write_file` or `edit_file`, the original is first copied to `GeminiForge/backups/`, and the result includes the backup ID.

- **Ask Gemini to restore**: tell it "restore the file you just changed"; it finds the ID with `list_backups` and restores with `restore_backup`.
- **Restore it yourself**: double-click `restore.bat`, which lists the 30 most recent backups; type an ID and press Enter.
- For a file that did not exist and was created by Gemini, restoring deletes it.
- The current content is backed up before a restore too, so a wrong restore can itself be restored.
- Backups are kept for 30 days, up to 5000 entries; files over 50 MB are only logged, their content is not backed up.
- **Files changed by commands run with `run` are not backed up** (for example deleting files in PowerShell, or `git checkout`).

## Tips

- **While you are using the computer too**: click, type, and key use the real mouse and keyboard and may fight with you. click_element triggers most buttons directly and does not get in your way.
- **Multiple monitors**: screenshots capture the monitor the mouse is on by default; after clicking an element or switching windows, the monitor that window is on is captured automatically.

## Safety notes

- **Emergency stop for mouse actions**: fling the mouse hard into any corner of the screen and the action in progress stops immediately.
- Gemini reads the web pages and files it looks at, and they can hide malicious instructions. Be especially careful with "Run all automatically".
- The server only accepts requests from the Chrome extension on this computer; ordinary web pages cannot call it.
- The files and screenshots you read are sent to Google's Gemini.

## Folder layout

```
GeminiForge/
├─ start.bat              starts the server for the extension (Option 1)
├─ start_mcp.bat          starts the MCP server (Option 2)
├─ autostart_on.bat       turns on start with Windows
├─ autostart_off.bat      turns off start with Windows
├─ restore.bat            lists backups and restores files
├─ *.sh                   the same scripts for macOS and Linux (start.sh, ...)
├─ backups/               automatic backups (not in git)
├─ server/               local server (Rust; one program for the extension and MCP)
│  ├─ src/                every tool, the extension server, the MCP server, approval window, backups
│  ├─ tests/
│  ├─ Cargo.toml
│  ├─ exe.cmd             finds the server program, builds it on first run
│  ├─ lang.cmd            localized messages shared by the batch files
│  └─ README.md           build, environment variables, fast waits
└─ extension/             Chrome extension
   ├─ manifest.json
   ├─ background.js       forwards requests to the local server
   ├─ content.js          on the Gemini page: panel, picking up tool commands, sending results back
   ├─ languages.js        language tag mapping
   ├─ _locales/           localized UI messages (en, zh_TW, zh_CN, ja, ko)
   └─ prompts/            localized tool instructions for Gemini (such as en.md; editable)
```

## FAQ

- **The dot is red**: `start.bat` is not running or was closed.
- **Port 18765 is in use**: add `set GF_PORT=<another port>` on the line before `call "%~dp0server\exe.cmd"` in `start.bat`, change 18765 in `extension/manifest.json` and `extension/background.js` as well, then reload the extension in `chrome://extensions`.
- **You changed the extension code (content.js and so on)**: click reload ↻ on the GeminiForge card in `chrome://extensions`, then refresh Gemini.
- **The extension stopped working after a Gemini update**: the structure of the Gemini page changed; the parts of `content.js` that find the message field and the send button need updating.
- **Can @GeminiForge run fully automatically, with no confirmations?** Only partly:
  - **The approval window on the computer**: can be turned off. Set `approval` to `none` in `server/mcp_config.json` and restart the server.
  - **"Allow / Deny" on the Gemini page**: cannot be turned off. It is Google's safety mechanism, and tools that change something (opening programs, clicking, changing files) are asked about every time.
  - You can skip the web confirmation by reporting every tool as read-only: set `"web_confirm": "off"` in `server/mcp_config.json` and restart the server (the default is `on`). **At your own risk**: this bypasses Google's safety design; web pages or files Gemini reads can hide malicious instructions (such as "delete this folder"), and without a confirmation they run on your computer right away. Use it only while you are watching.
  - The closest to fully automatic: use @GeminiForge with `approval` set to `none` and click "Allow" on the Gemini page once for each step that changes something; or use the Chrome extension (Chat mode) with "Run all automatically", which needs no clicks at all but is best used only while you are watching.
