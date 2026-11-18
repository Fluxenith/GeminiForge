# GeminiForge

[English](README.md) | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md)

Turn Gemini Web into a local computer agent — files, terminal, and desktop control.

讓網頁版 Gemini（gemini.google.com）能操作你的電腦：讀寫檔案、執行指令、寫程式、看螢幕並控制滑鼠鍵盤。
用的是你網頁版 Gemini 的額度，不需要 API key。

## 環境需求

- Windows 10 / 11
- Python 3.10 以上版本，安裝時勾選「Add Python to PATH」
- 可登入使用的 Gemini 帳號
- Chrome 外掛模式需要 Google Chrome；MCP 模式需要安裝並登入 Tailscale，且帳號可使用 Funnel

下載本專案後，將整個資料夾解壓縮到固定位置，再依下方任一方式設定。

外掛面板、桌面確認視窗、伺服器視窗和 .bat 腳本都會使用外掛面板中選擇的語言（預設為 Windows 顯示語言）。

```
gemini.google.com  ──(Gemini 輸出工具指令)──▶  Chrome 外掛  ──▶  本機伺服器 127.0.0.1:18765
        ▲                                                              │ 讀寫檔案 / PowerShell / 截圖 / 滑鼠鍵盤
        └──────────────(結果和截圖自動貼回對話並送出)◀──────────────────┘
```

## 方式一：Chrome 外掛

### 第一次設定

1. **啟動本機伺服器**：雙擊 `start.bat`。第一次會自動安裝套件。黑色視窗要保持開著，關掉就停止。
2. **安裝 Chrome 外掛**：
   1. Chrome 網址列輸入 `chrome://extensions`
   2. 打開右上角的「開發人員模式」
   3. 按「載入未封裝項目」，選擇這個資料夾裡的 `extension` 資料夾
3. **打開 Gemini**：到 gemini.google.com（已開著的請重新整理），右下角會出現 **⚒ GeminiForge** 面板，圓點變綠色代表已連上伺服器。
4. **第一次上傳截圖**：按面板上的「測試截圖」。如果 Gemini 跳出「根據圖像和檔案建立內容」的視窗，按「同意」（只需要一次）。測試的截圖放進輸入框後，按附件上的 ✕ 移除即可。

### 使用方式

在 Gemini 開一個**新對話**，直接打你的需求送出就好，例如「幫我在桌面建立一個 hello.py 並執行它」。
外掛會自動在第一則訊息前面帶入工具說明（面板上「新對話自動帶入工具說明」打勾時）。

之後 Gemini 回覆裡的工具指令會被外掛抓到，依照模式執行（或等你按「執行」），結果自動傳回給 Gemini，一直重複到任務完成。

想在已經開始的對話中使用，可以按面板的「插入工具說明」。

右下角的執行模式只控制外掛的 `gf_tool` JSON 指令。若畫面中央出現「Let Gemini use … from GeminiForge」及「允許／拒絕」，代表該輪使用 MCP 連接器；外掛的「全部自動執行」不會取消這層確認。要使用外掛模式，請建立新對話、選「對話」模式，不要標記 `@GeminiForge`，並保持「新對話自動帶入工具說明」勾選。

**用 Gem 也可以**：在 Gemini 左側「Gem 管理工具」建立新的 Gem，把 `extension/prompts/zh-TW.md` 的內容貼到「指示」裡。在 Gem 裡的對話外掛不會再重複帶入說明。

**建議用 Pro 或「思考型」模型**：多步驟的任務（寫程式、操作電腦）Pro 比 Flash 穩定很多。

## 方式二：@ 自訂應用程式（MCP，建議）

透過 Gemini 的自訂應用程式連接 MCP，在輸入框選取 `@GeminiForge` 即可使用。不需要 Chrome 外掛，同樣使用網頁版額度。自訂應用程式是否可用，會依 Gemini 帳號、介面語言與模式而異。

```
gemini.google.com ──(MCP，OAuth 驗證)──▶ https://<你的電腦>.ts.net:10000 ──Tailscale Funnel──▶ 127.0.0.1:18766 (mcp_server.py)
```

### 第一次設定

1. **啟動 MCP 伺服器**：雙擊 `start_mcp.bat`。視窗會顯示 **MCP 網址** 和 **配對碼**，要保持開著。
2. **開公開通道**（只需做一次）：用 PowerShell 執行
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --bg --https=10000 http://127.0.0.1:18766
   ```
   Funnel 會顯示這台電腦的公開網址，在網址後面加上 `/mcp` 即為連接器網址，例如 `https://<裝置名稱>.<tailnet>.ts.net:10000/mcp`。請使用你自己的網址。要關掉通道：
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --https=10000 off
   ```
3. **在 Gemini 加入**：Gemini 輸入框打 `@` → 查看所有連接器 → 建立自訂應用程式 → 貼上 MCP 網址 → 繼續。
   - 「其他設定」的用戶端 ID／密鑰可以先留空。如果 Gemini 要求填寫，請填 `server/mcp_config.json` 裡的 `client_id` 和 `client_secret`。
4. 跳出 **連結 GeminiForge** 頁面時，輸入伺服器視窗上的 **配對碼**，按「允許連結」。

### 使用

1. 用**英文介面**開 Gemini：`https://gemini.google.com/app?hl=en`（或用 Gemini 桌面 App）。英文介面的一般對話（Chat）就能用 @GeminiForge，甚至不打 @ 也會自動呼叫。
   中文介面的「對話」模式 @ 選單裡不會出現自訂應用程式，只能切到「**任務**」模式使用（2026-11 實測）。
2. 輸入 `@`，從選單選 **GeminiForge**，再打你的需求，例如「列出我桌面上的檔案」。

會改變東西的動作，**Gemini 網頁會先問一次**「Let Gemini use … from GeminiForge」，按「允許」才會執行。

接著伺服器還會在你的電腦上跳出**第二道確認視窗**（`approval` 預設 `readonly` 時）：
- **允許**：只允許這一次
- **30 分鐘內都允許這個工具**：之後同一種動作不再問
- **拒絕**，或 45 秒沒按就自動拒絕（Gemini 大約等一分鐘就會放棄）

確認模式可以在 `server/mcp_config.json` 的 `approval` 修改後重啟伺服器：`readonly`（預設：Gemini 網頁確認 + 電腦確認視窗）、`all`（連唯讀動作也跳視窗）、`none`（只靠 Gemini 網頁的確認，不跳電腦視窗）。

### 安全

- 必須輸入配對碼才能連結；錯 5 次會鎖 10 分鐘。
- `server/mcp_config.json`（配對碼、密鑰）和 `server/mcp_auth.json`（登入憑證）不要分享給別人。
- 想讓所有已連結的裝置失效：關掉伺服器，刪除 `server/mcp_auth.json`，再重新啟動。想換配對碼：刪除 `mcp_config.json` 裡的 `pairing_code` 那一行再重啟。
- 通道一打開，任何人都連得到這個網址，但沒有配對碼就無法使用任何工具。

## 面板說明

在 Gemini 頁面右下角展開 **⚒ GeminiForge**，用「語言」選單切換 **English、繁體中文、简体中文、日本語、한국어**。首次開啟時依 Chrome 的介面語言顯示，其他語言預設使用英文。選擇會保存在目前的 Chrome 個人資料，並同步到已開啟的 Gemini 分頁。

面板、確認按鈕及之後插入的工具說明都會使用所選語言。本機伺服器連線後，下一次開啟的 MCP 配對頁和桌面確認視窗也會套用設定。更新現有安裝時，請重新載入外掛、重新整理 Gemini 分頁，並重新啟動 `start.bat` 的本機伺服器。如果也使用 MCP，請一併重新啟動 `start_mcp.bat`。

| 項目 | 作用 |
|---|---|
| 圓點 | 綠色：伺服器已連線；紅色：請執行 `start.bat` |
| 啟用 | 關掉後外掛不會執行任何工具 |
| 新對話自動帶入工具說明 | 在新對話送出第一則訊息時，自動把工具說明加在前面 |
| 模式 | **每一步都要確認**：最安全<br>**唯讀自動，其餘確認**（預設）：看檔案、截圖自動執行；寫檔、執行指令、點滑鼠要你按「執行」<br>**全部自動執行**：完全放手，請只在你信任的任務使用 |
| 停止 | 立刻中止目前的流程 |
| 確認框的「本對話都允許」 | 同一個對話裡，這個工具之後不再詢問（換對話就重置） |
| 測試截圖 | 檢查截圖和上傳是否正常 |

連續自動執行 40 輪後會停下來問你要不要繼續。

## 開機自動啟動（可選）

- 雙擊 `autostart_on.bat`：之後開機會自動以最小化視窗啟動伺服器
- 雙擊 `autostart_off.bat`：取消

## Gemini 能用的工具

- **檔案與程式**：list_dir、read_file、write_file、edit_file、search、find_files、run（PowerShell）、start_process、open、剪貼簿
- **備份與復原**：list_backups、restore_backup（見下方「檔案備份」）
- **操作電腦**：screenshot、zoom（放大看小字）、click、double_click、right_click、move、drag、type、key、scroll
- **精準點擊**：list_windows、focus_window、ui_elements（用 Windows 無障礙介面列出按鈕和連結的名稱與位置）、click_element（用名稱或編號點擊，會直接觸發按鈕：不移動你的滑鼠、視窗在背景也可以，並回報視窗顯示的文字）

`ui_elements` 會回傳 `window_id`，後續操作帶上同一個 ID，能避免選到同名的瀏覽器分頁。已觀察到的一串按鈕可用 `click_elements {window_id, names}` 依序執行（最多 16 個）；任一步失敗即停止。MCP 模式仍會由 Gemini 網頁確認這一整串動作，並不會取消網頁端的確認。視窗或元素失效時，工具會回報錯誤並要求重新觀察，不會沿用舊座標點擊。

工具說明在 `extension/prompts/`，每種語言一個檔案，可以自行修改。

## 檔案備份

Gemini 每次用 `write_file` 或 `edit_file` 改檔案前，會先把原檔複製到 `GeminiForge/backups/`，並在結果裡附上備份編號。

- **請 Gemini 復原**：跟它說「把剛剛改的檔案復原」，它會用 `list_backups` 找編號、`restore_backup` 復原。
- **自己復原**：雙擊 `restore.bat`，會列出最近 30 筆備份，輸入編號按 Enter 就好。
- 原本不存在、由 Gemini 新建的檔案，復原會把它刪掉。
- 復原前也會先備份目前的內容，所以復原錯了可以再復原回來。
- 保留 30 天、最多 5000 筆；超過 50 MB 的檔案只記錄、不備份內容。
- **`run` 執行的指令改到的檔案不會備份**（例如 PowerShell 刪檔、`git checkout`）。

## 小提醒

- **你也在用電腦時**：click、type、key 會用真的滑鼠鍵盤，可能和你搶操作。click_element 對大部分按鈕是直接觸發，不會干擾你。
- **多螢幕**：截圖預設拍滑鼠所在的螢幕；點擊元素或切換視窗後，會自動拍那個視窗所在的螢幕。

## 安全須知

- **緊急停止滑鼠操作**：把滑鼠用力甩到螢幕任一角落，正在進行的操作會立刻中止。
- Gemini 會讀到它看過的網頁和檔案內容，這些內容可能藏有惡意指示。用「全部自動執行」時要特別小心。
- 伺服器只接受本機上 Chrome 外掛的請求，一般網頁無法呼叫它。
- 你讀取的檔案和截圖內容會傳到 Google 的 Gemini。

## 資料夾結構

```
GeminiForge/
├─ start.bat              啟動外掛用的伺服器（方式一）
├─ start_mcp.bat          啟動 MCP 伺服器（方式二）
├─ autostart_on.bat       設定開機自動啟動
├─ autostart_off.bat      取消開機自動啟動
├─ restore.bat            列出備份並復原檔案
├─ backups/               自動備份（不進 git）
├─ server/
│  ├─ server.py           所有工具的實作，也是外掛用的伺服器
│  ├─ mcp_server.py       MCP 伺服器（OAuth 驗證 + 確認視窗）
│  ├─ approve_dialog.py   電腦上的確認視窗
│  ├─ backups.py          改檔前自動備份、復原
│  └─ requirements.txt
└─ extension/             Chrome 外掛
   ├─ manifest.json
   ├─ background.js       把請求轉給本機伺服器
   ├─ content.js          在 Gemini 網頁上：面板、抓工具指令、回傳結果
   └─ prompts/            給 Gemini 的工具說明，每種語言一個檔案（可自行修改）
      └─ zh-TW.md、zh-CN.md、en.md、ja.md、ko.md
```

## 常見問題

- **圓點是紅色**：`start.bat` 沒開或被關掉了。
- **埠 18765 被佔用**：在 `start.bat` 的 `set PYTHONIOENCODING` 下一行加 `set GF_PORT=其他埠號`，並把 `extension/manifest.json`、`extension/background.js` 裡的 18765 一起改掉，再到 `chrome://extensions` 重新載入外掛。
- **改了外掛程式碼（content.js 等）**：到 `chrome://extensions` 按 GeminiForge 卡片上的重新載入 ↻，再重新整理 Gemini。
- **Gemini 改版後外掛失效**：Gemini 網頁結構變了，需要更新 `content.js` 裡找輸入框、傳送按鈕的部分。
- **@GeminiForge 能不能完全自動、不用按確認？** 只能做到一部分：
  - **電腦上的確認視窗**：可以關。把 `server/mcp_config.json` 的 `approval` 改成 `none` 再重啟伺服器。
  - **Gemini 網頁上的「允許 / 拒絕」**：關不掉。這是 Google 的安全機制，會改東西的工具（開程式、點擊、改檔案）每次都會問。
  - 可以把所有工具回報成「只會讀取」來跳過網頁確認：在 `server/mcp_config.json` 設 `"web_confirm": "off"` 再重啟伺服器（預設 `on`）。**風險自負**：這等於繞過 Google 的安全設計；Gemini 讀到的網頁或檔案裡可能藏有惡意指令（例如「刪掉某資料夾」），沒有確認就會直接在電腦上執行。建議只在你看著的時候使用。
  - 最接近全自動的做法：用 @GeminiForge 並把 `approval` 設為 `none`，每個會改東西的步驟在 Gemini 網頁按一次「允許」；或改用 Chrome 外掛（對話模式）的「全部自動執行」，完全不用按，但最好只在你看著的時候用。

## 授權

由 [Fluxenith](https://github.com/Fluxenith) 維護。MIT 授權，詳見 [LICENSE](LICENSE)。本專案與 Google 無隸屬或背書關係。
