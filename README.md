# GeminiForge

[English](README.md) | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md)

Turn Gemini Web into a local computer agent — files, terminal, and desktop control.

Lets the web version of Gemini (gemini.google.com) operate your computer: read and write files, run commands, write code, see the screen, and control the mouse and keyboard.
It uses your Gemini web quota; no API key is needed.

## Requirements

- Windows 10 / 11
- Python 3.10 or later; check "Add Python to PATH" during installation
- A Gemini account you can sign in with
- Chrome extension mode requires Google Chrome; MCP mode requires Tailscale installed and signed in, with an account that can use Funnel

After downloading the project, extract the whole folder to a permanent location, then set it up using either method below.

The extension panel, the desktop approval popup, the server windows and the .bat scripts follow the language chosen in the extension panel (defaulting to the Windows display language).

```
gemini.google.com  ──(Gemini outputs tool commands)──▶  Chrome extension  ──▶  local server 127.0.0.1:18765
        ▲                                                              │ files / PowerShell / screenshots / mouse & keyboard
        └──────────(results and screenshots pasted back into the chat and sent)◀──────────┘
```

## Method 1: Chrome extension

### First-time setup

1. **Start the local server**: double-click `start.bat`. The first run installs the required packages automatically. Keep the black window open; closing it stops the server.
2. **Install the Chrome extension**:
   1. Type `chrome://extensions` in the Chrome address bar
   2. Turn on "Developer mode" in the top-right corner
   3. Click "Load unpacked" and select the `extension` folder inside this folder
3. **Open Gemini**: go to gemini.google.com (reload it if it is already open). The **⚒ GeminiForge** panel appears in the bottom-right corner; a green dot means it is connected to the server.
4. **First screenshot upload**: click "Test screenshot" on the panel. If Gemini shows a dialog about creating content from images and files, click "Agree" (only needed once). Once the test screenshot is in the input box, click the ✕ on the attachment to remove it.

### Usage

Start a **new chat** in Gemini and just type your request, for example "Create hello.py on my desktop and run it".
The extension automatically prepends the tool instructions to the first message (when "Add tool instructions to new chats" is checked on the panel).

From then on, the extension picks up tool commands in Gemini's replies, runs them according to the mode (or waits for you to click "Run"), and sends the results back to Gemini automatically, repeating until the task is done.

To use it in a chat that has already started, click "Insert tool instructions" on the panel.

The execution mode in the bottom-right corner only controls the extension's `gf_tool` JSON commands. If "Let Gemini use … from GeminiForge" with "Allow / Deny" appears in the middle of the screen, that turn is using the MCP connector; the extension's "Run all automatically" does not remove that confirmation. To use extension mode, start a new chat, choose Chat mode, do not tag `@GeminiForge`, and keep "Add tool instructions to new chats" checked.

**Gems work too**: in Gemini's left sidebar, open the Gem manager, create a new Gem, and paste the contents of `extension/prompts/en.md` into its "Instructions". In chats inside that Gem, the extension will not add the instructions again.

**Pro or a "Thinking" model is recommended**: for multi-step tasks (writing code, operating the computer), Pro is much more reliable than Flash.

## Method 2: @ custom app (MCP, recommended)

Connect over MCP through Gemini's custom apps, then select `@GeminiForge` in the input box. No Chrome extension is needed, and it also uses your web quota. Whether custom apps are available depends on your Gemini account, interface language and mode.

```
gemini.google.com ──(MCP, OAuth)──▶ https://<your-computer>.ts.net:10000 ──Tailscale Funnel──▶ 127.0.0.1:18766 (mcp_server.py)
```

### First-time setup

1. **Start the MCP server**: double-click `start_mcp.bat`. The window shows the **MCP URL** and the **pairing code**; keep it open.
2. **Open a public tunnel** (only once): run in PowerShell
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --bg --https=10000 http://127.0.0.1:18766
   ```
   Funnel shows this computer's public URL. Append `/mcp` to it to get the connector URL, for example `https://<device-name>.<tailnet>.ts.net:10000/mcp`. Use your own URL. To close the tunnel:
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --https=10000 off
   ```
3. **Add it in Gemini**: type `@` in the Gemini input box → view all connectors → create a custom app → paste the MCP URL → continue.
   - You can leave the client ID / secret under the additional settings empty at first. If Gemini asks for them, enter `client_id` and `client_secret` from `server/mcp_config.json`.
4. When the **Connect to GeminiForge** page appears, enter the **pairing code** shown in the server window and click "Allow connection".

### Usage

1. Open Gemini with the **English UI**: `https://gemini.google.com/app?hl=en` (or use the Gemini desktop app). In the English UI, @GeminiForge works in regular Chat, and Gemini may even call it automatically without typing @.
   In the Chinese UI, custom apps do not appear in the @ menu in 「對話」 (Chat) mode; you can only use them by switching to 「**任務**」 mode (tested 2026-11).
2. Type `@`, choose **GeminiForge** from the menu, then type your request, for example "List the files on my desktop".

For actions that change things, **the Gemini web page asks once first**: "Let Gemini use … from GeminiForge". It runs only after you click "Allow".

The server then shows a **second confirmation window** on your computer (when `approval` is the default `readonly`):
- **Allow**: allow this one time only
- **Allow this tool for 30 minutes**: the same kind of action will not ask again
- **Deny**, or it denies automatically after 45 seconds without a click (Gemini gives up after about a minute)

To change the confirmation mode, edit `approval` in `server/mcp_config.json` and restart the server: `readonly` (default: Gemini web confirmation + desktop confirmation window), `all` (popup even for read-only actions), `none` (rely only on Gemini's web confirmation, no desktop popup).

### Security

- Connecting requires the pairing code; 5 wrong attempts lock it for 10 minutes.
- Do not share `server/mcp_config.json` (pairing code, secret) or `server/mcp_auth.json` (sign-in credentials) with anyone.
- To revoke all connected devices: stop the server, delete `server/mcp_auth.json`, and start it again. To change the pairing code: delete the `pairing_code` line in `mcp_config.json` and restart.
- Once the tunnel is open, anyone can reach the URL, but no tool can be used without the pairing code.

## Panel guide

Expand **⚒ GeminiForge** in the bottom-right corner of the Gemini page and use the "Language" menu to switch between **English, 繁體中文, 简体中文, 日本語, 한국어**. On first open it follows Chrome's interface language; other languages default to English. The choice is saved in the current Chrome profile and synced to open Gemini tabs.

The panel, the confirmation buttons and any tool instructions inserted afterwards use the selected language. Once the local server is connected, the next MCP pairing page and desktop confirmation window will also use this setting. When updating an existing installation, reload the extension, refresh the Gemini tab, and restart the local server from `start.bat`. If you also use MCP, restart `start_mcp.bat` as well.

| Item | What it does |
|---|---|
| Dot | Green: server connected; red: run `start.bat` |
| Enable | When off, the extension runs no tools |
| Add tool instructions to new chats | Automatically prepends the tool instructions when you send the first message in a new chat |
| Mode | **Ask at every step**: safest<br>**Ask except for read-only tools** (default): viewing files and screenshots run automatically; writing files, running commands and mouse clicks wait for you to click "Run"<br>**Run all automatically**: fully hands-off; use only for tasks you trust |
| Stop | Immediately aborts the current flow |
| "Allow for this chat" in the confirmation box | This tool will not ask again in the same chat (resets in a new chat) |
| Test screenshot | Checks that screenshots and uploads work |

After 40 consecutive automatic rounds, it stops and asks whether to continue.

## Start automatically at boot (optional)

- Double-click `autostart_on.bat`: the server starts automatically in a minimized window at boot
- Double-click `autostart_off.bat`: cancel

## Tools available to Gemini

- **Files and programs**: list_dir, read_file, write_file, edit_file, search, find_files, run (PowerShell), start_process, open, clipboard
- **Backup and restore**: list_backups, restore_backup (see "File backups" below)
- **Computer control**: screenshot, zoom (magnify small text), click, double_click, right_click, move, drag, type, key, scroll
- **Precise clicking**: list_windows, focus_window, ui_elements (lists names and positions of buttons and links via Windows UI Automation), click_element (clicks by name or index and triggers the button directly: it does not move your mouse, works even when the window is in the background, and reports the text shown in the window)

`ui_elements` returns a `window_id`; passing the same ID in later actions avoids picking a browser tab with the same name. A series of buttons you have already observed can be run in order with `click_elements {window_id, names}` (up to 16); it stops at the first failing step. In MCP mode, Gemini's web page still confirms the whole series; it does not remove the web-side confirmation. If a window or element is no longer valid, the tool reports an error and asks to observe again rather than clicking stale coordinates.

The tool instructions are in `extension/prompts/`, one file per language, and you can edit them.

## File backups

Each time Gemini changes a file with `write_file` or `edit_file`, it first copies the original to `GeminiForge/backups/` and includes a backup ID in the result.

- **Ask Gemini to restore**: tell it "restore the file you just changed"; it will find the ID with `list_backups` and restore with `restore_backup`.
- **Restore yourself**: double-click `restore.bat`; it lists the 30 most recent backups. Type an ID and press Enter.
- Restoring a file that did not exist before (newly created by Gemini) deletes it.
- The current content is also backed up before restoring, so a wrong restore can be undone.
- Backups are kept for 30 days, up to 5000 entries; files over 50 MB are only recorded, not backed up.
- **Files changed by commands run with `run` are not backed up** (for example, deleting files in PowerShell, `git checkout`).

## Tips

- **While you are also using the computer**: click, type and key use the real mouse and keyboard and may fight with you. click_element triggers most buttons directly and will not interfere with you.
- **Multiple monitors**: screenshots capture the monitor the mouse is on by default; after clicking an element or switching windows, it automatically captures the monitor that window is on.

## Safety notes

- **Emergency stop for mouse control**: fling the mouse hard into any corner of the screen and the ongoing action stops immediately.
- Gemini reads the web pages and files it views, and that content may hide malicious instructions. Be especially careful with "Run all automatically".
- The server only accepts requests from the Chrome extension on this computer; ordinary web pages cannot call it.
- The contents of files you read and screenshots are sent to Google's Gemini.

## Folder structure

```
GeminiForge/
├─ start.bat              Starts the server for the extension (Method 1)
├─ start_mcp.bat          Starts the MCP server (Method 2)
├─ autostart_on.bat       Enables start at boot
├─ autostart_off.bat      Disables start at boot
├─ restore.bat            Lists backups and restores files
├─ backups/               Automatic backups (not in git)
├─ server/
│  ├─ server.py           Implements all tools; also the server for the extension
│  ├─ mcp_server.py       MCP server (OAuth + confirmation window)
│  ├─ approve_dialog.py   Confirmation window on the computer
│  ├─ backups.py          Automatic backup before edits, and restore
│  └─ requirements.txt
└─ extension/             Chrome extension
   ├─ manifest.json
   ├─ background.js       Forwards requests to the local server
   ├─ content.js          On the Gemini page: panel, captures tool commands, returns results
   └─ prompts/            Tool instructions for Gemini, one file per language (editable)
      └─ zh-TW.md, zh-CN.md, en.md, ja.md, ko.md
```

## FAQ

- **The dot is red**: `start.bat` is not running or was closed.
- **Port 18765 is in use**: in `start.bat`, add `set GF_PORT=<another port>` on the line after `set PYTHONIOENCODING`, change 18765 in `extension/manifest.json` and `extension/background.js` as well, then reload the extension at `chrome://extensions`.
- **Changed the extension code (content.js, etc.)**: at `chrome://extensions`, click reload ↻ on the GeminiForge card, then refresh Gemini.
- **The extension stops working after a Gemini update**: Gemini's page structure changed; the part of `content.js` that finds the input box and send button needs updating.
- **Can @GeminiForge be fully automatic, with no confirmations?** Only partly:
  - **The confirmation window on the computer**: can be turned off. Set `approval` in `server/mcp_config.json` to `none` and restart the server.
  - **"Allow / Deny" on the Gemini web page**: cannot be turned off. It is Google's safety mechanism; tools that change things (opening programs, clicking, editing files) ask every time.
  - You can skip the web confirmation by reporting every tool as "read-only": set `"web_confirm": "off"` in `server/mcp_config.json` and restart the server (default `on`). **At your own risk**: this bypasses Google's safety design; web pages or files Gemini reads may hide malicious instructions (for example "delete a certain folder"), which would run on your computer without confirmation. Use it only while you are watching.
  - The closest to fully automatic: use @GeminiForge with `approval` set to `none` and click "Allow" once on the Gemini web page for each step that changes things; or use the Chrome extension (Chat mode) with "Run all automatically", which needs no clicks at all, but is best used only while you are watching.

## License

Maintained by [Fluxenith](https://github.com/Fluxenith). MIT licensed; see [LICENSE](LICENSE). This project is not affiliated with or endorsed by Google.
