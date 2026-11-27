[English](README.md) | [繁體中文](README.zh-TW.md) | **简体中文** | [日本語](README.ja.md) | [한국어](README.ko.md)

# GeminiForge

让网页版 Gemini（gemini.google.com）能操作你的电脑：读写文件、运行命令、写代码、看屏幕并控制鼠标键盘。
用的是你网页版 Gemini 的额度，不需要 API key。

```
gemini.google.com  ──(Gemini 输出工具指令)──▶  Chrome 插件  ──▶  本机服务器 127.0.0.1:18765
        ▲                                                              │ 读写文件 / PowerShell / 截屏 / 鼠标键盘
        └──────────────(结果和截屏自动贴回对话并发送)◀──────────────────┘
```

## 下载

到 [Releases](../../releases/latest) 下载 `GeminiForge-<版本>-windows-x64.zip`（Windows），解压到任意文件夹。里面已附带构建好的服务器（`server\geminiforge-server.exe`），无需另外安装任何东西。首次运行时如果 Windows 弹出 SmartScreen 警告，点「更多信息」→「仍要运行」。

想从源代码自行构建的话，先安装 [Rust](https://rustup.rs)，`start.bat` 首次运行时会自动构建（大约一分钟）。

Mac 或 Linux 电脑请看下一节。

## macOS 与 Linux

Chrome 插件（方式一）、MCP（方式二）以及文件和命令工具在 macOS 和 Linux 上也能用。Gemini 可以用 `screenshot` 看屏幕，但操作鼠标键盘目前只有 Windows 有；GeminiForge 会自动告诉 Gemini。

1. 到 [Releases](../../releases/latest) 下载你电脑的压缩包：`macos-arm64`（Apple 芯片）、`macos-x64`（Intel Mac）、`linux-x64` 或 `linux-arm64`，解压。
2. 改在终端运行 `.sh` 脚本，代替 `.bat` 文件：
   ```
   cd GeminiForge
   sh start.sh            # 方式一：插件用的服务器
   sh start_mcp.sh        # 方式二：MCP 服务器
   sh restore.sh          # 恢复备份
   sh autostart_on.sh     # 登录时自动启动（autostart_off.sh 取消）
   ```
   使用期间终端保持打开。方式一和方式二的其他步骤都一样。

- `run` 用 bash，不是 PowerShell。
- **macOS**：首次启动可能出现“无法打开，因为无法验证开发者”。到“系统设置”→“隐私与安全性”，对 geminiforge-server 点“仍要打开”，再启动一次。
- **Linux**：MCP 的确认窗口需要 `zenity`（或 `kdialog`），剪贴板工具需要 `wl-clipboard`（Wayland）或 `xclip`（X11），例如 `sudo apt install zenity wl-clipboard xclip`。
- **截图**：macOS 要允许启动 GeminiForge 的终端录制屏幕（“系统设置”→“隐私与安全性”→“屏幕录制”）。Linux 需要安装 `gnome-screenshot`、`grim`、`spectacle`、`scrot` 或 ImageMagick 其中一个。
- 方式二的 Tailscale 通道：`tailscale funnel --bg --https=10000 http://127.0.0.1:18767`（Mac App Store 版的命令是 `/Applications/Tailscale.app/Contents/MacOS/Tailscale`）。
- 想从源代码构建，先安装 [Rust](https://rustup.rs)，`start.sh` 首次运行时会自动构建。

## 方式一：Chrome 插件

### 第一次设置

1. **启动本机服务器**：双击 `start.bat`。黑色窗口要保持打开，关掉就停止。
2. **安装 Chrome 插件**：
   1. 在 Chrome 地址栏输入 `chrome://extensions`
   2. 打开右上角的「开发者模式」
   3. 点击「加载未打包的扩展程序」，选择这个文件夹里的 `extension` 文件夹
3. **打开 Gemini**：到 gemini.google.com（已经开着的请刷新），右下角会出现 **⚒ GeminiForge** 面板，圆点变绿色表示已连上服务器。
4. **第一次上传截屏**：点击面板上的「测试截屏」。如果 Gemini 弹出关于根据图片和文件创建内容的同意窗口，请点同意（只需要一次）。测试的截屏放进输入框后，点附件上的 ✕ 移除即可。

### 使用方式

在 Gemini 开一个**新对话**，直接输入你的需求并发送，例如「帮我在桌面创建一个 hello.py 并运行它」。
插件会自动在第一则消息前面加上工具说明（面板上「新对话自动添加工具说明」勾选时）。

之后 Gemini 回复里的工具指令会被插件抓到，按照运行模式运行（或等你点「运行」），结果自动传回给 Gemini，一直重复到任务完成。

想在已经开始的对话中使用，可以点面板的「插入工具说明」。

右下角的运行模式只控制插件的 `gf_tool` JSON 指令。如果页面中央出现「Let Gemini use … from GeminiForge」和「允许／拒绝」，表示该轮使用的是 MCP 连接器；插件的「全部自动运行」不会取消这层确认。要使用插件模式，请新建对话、选「对话」模式，不要标记 `@GeminiForge`，并保持「新对话自动添加工具说明」勾选。

**用 Gem 也可以**：在 Gemini 左侧的 Gem 页面新建一个 Gem，把 `extension/prompts/zh-CN.md`（或其他语言对应的 `extension/prompts/<语言>.md`）的内容粘贴到「指令」里。在 Gem 里的对话插件不会再重复添加说明。

**建议用 Pro 或「思考型」模型**：多步骤的任务（写代码、操作电脑）Pro 比 Flash 稳定很多。

## 方式二：@ 自定义应用（MCP，推荐）

这是 Gemini 官方的工具连接方式：在 Gemini 输入框输入 `@GeminiForge` 就能用。不需要 Chrome 插件，也不怕 Gemini 改版，同样使用网页版额度。

```
gemini.google.com ──(MCP，OAuth 验证)──▶ https://<你的电脑>.ts.net:10000 ──Tailscale Funnel──▶ 127.0.0.1:18767 (geminiforge-server.exe mcp)
```

### 第一次设置

1. **启动 MCP 服务器**：双击 `start_mcp.bat`。窗口会显示 **MCP 地址** 和 **配对码**，要保持打开。
2. **打开公开通道**（只需做一次）：用 PowerShell 运行
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --bg --https=10000 http://127.0.0.1:18767
   ```
   Funnel 会显示这台电脑的公开地址，在地址后面加上 `/mcp` 就是连接器地址，例如 `https://<设备名称>.<tailnet>.ts.net:10000/mcp`。请使用你自己的地址。要关掉通道：
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --https=10000 off
   ```
   没有 Tailscale？安装 [cloudflared](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/)，再把 `server/mcp_config.json` 的 `"tunnel"` 改成 `"cloudflared"`：MCP 服务器会自己打开快速通道并显示地址。这个地址每次启动服务器都会变，所以每次启动后要把新地址填进 Gemini 的自定义应用，再配对一次。
3. **在 Gemini 添加**：在 Gemini 输入框输入 `@` → 查看所有连接器 → 创建自定义应用 → 粘贴 MCP 地址 → 继续。
   - 「其他设置」里的客户端 ID／密钥可以先留空。如果 Gemini 要求填写，请填 `server/mcp_config.json` 里的 `client_id` 和 `client_secret`。
4. 弹出 **连接 GeminiForge** 页面时，输入服务器窗口上的 **配对码**，点「允许连接」。

### 使用

1. 打开 Gemini，两种都可以：
   - **Gemini 桌面应用**（Windows）：非英文界面的「对话」模式也能用，`@` 菜单里直接有 GeminiForge，不必切换英文界面或「任务」模式（2026-11-23 以应用 1.21.1 实测）。桌面应用只能用这个方式，方式一的 Chrome 插件无法加载到桌面应用里。
   - **网页版**：用**英文界面** `https://gemini.google.com/app?hl=en`。英文界面的普通对话（Chat）就能用 @GeminiForge，甚至不输入 @ 也会自动调用。
     非英文界面的「对话」模式 @ 菜单里不会出现自定义应用，只能切到「**任务**」模式使用（2026-11 实测）。
2. 输入 `@`，从菜单选 **GeminiForge**，再输入你的需求，例如「列出我桌面上的文件」。

会改变东西的操作，**Gemini 网页会先问一次**「Let Gemini use … from GeminiForge」，点「允许」才会运行。

接着服务器还会在你的电脑上弹出**第二道确认窗口**（`approval` 为默认的 `readonly` 时）：
- **允许**：只允许这一次
- **30 分钟内都允许这个工具**：之后同一种操作不再询问
- **拒绝**，或 45 秒没点就自动拒绝（Gemini 大约等一分钟就会放弃）

确认模式可以在 `server/mcp_config.json` 的 `approval` 修改后重启服务器：`readonly`（默认：Gemini 网页确认 + 电脑确认窗口）、`all`（连只读操作也弹窗口）、`none`（只靠 Gemini 网页的确认，不弹电脑窗口）。

### 安全

- 必须输入配对码才能连接；错 5 次会锁定 10 分钟。
- `server/mcp_config.json`（配对码、密钥）和 `server/mcp_auth.json`（登录凭据）不要分享给别人。
- 想让所有已连接的设备失效：关掉服务器，删除 `server/mcp_auth.json`，再重新启动。想换配对码：删除 `mcp_config.json` 里的 `pairing_code` 那一行再重启。
- 通道一打开，任何人都能连到这个地址，但没有配对码就无法使用任何工具。

## 面板说明

在 Gemini 页面右下角展开 **⚒ GeminiForge**，用「语言」菜单切换 **English、繁體中文、简体中文、日本語、한국어**。首次打开时按 Chrome 的界面语言显示，其他语言默认使用英文。选择会保存在当前的 Chrome 个人资料，并同步到已打开的 Gemini 标签页。

面板、确认按钮以及之后插入的工具说明都会使用所选语言。本机服务器连接后，下一次打开的 MCP 配对页和桌面确认窗口也会套用设置。更新现有安装时，请重新加载插件、刷新 Gemini 标签页，并重新启动 `start.bat` 的本机服务器。如果也使用 MCP，请一并重新启动 `start_mcp.bat`。

| 项目 | 作用 |
|---|---|
| 圆点 | 绿色：服务器已连接；红色：请运行 `start.bat` |
| 启用 | 关掉后插件不会运行任何工具 |
| 新对话自动添加工具说明 | 在新对话发送第一则消息时，自动把工具说明加在前面 |
| 回复完成时通知 | Gemini 回复完成（或整个工具任务完成）、等你点「运行」、或流程出错时，弹出 Windows 通知；点通知会切回那个 Gemini 标签页。默认「仅在离开标签页时」：你切到别的标签页或程序才通知，正在看 Gemini 标签页时不通知。选「始终通知」连看着标签页也会通知，选「关闭」就完全不通知。旁边的「测试通知」会立刻发一则通知，用来确认 Windows 有没有拦截。每发出一则通知，面板记录会多一行 🔔。需要「启用」也勾选 |
| 运行模式 | **每一步都要确认**：最安全<br>**只读工具自动运行，其余需确认**（默认）：看文件、截屏自动运行；写文件、运行命令、点鼠标要你点「运行」<br>**全部自动运行**：完全放手，请只在你信任的任务使用 |
| 停止 | 立刻中止当前的流程 |
| 确认框的「本对话都允许」 | 同一个对话里，这个工具之后不再询问（换对话就重置） |
| 测试截屏 | 检查截屏和上传是否正常 |

连续自动运行 200 轮后会停下来问你要不要继续。

没看到通知时：到 Windows「设置 → 系统 → 通知」确认 **Google Chrome** 的通知是开的，并关掉「请勿打扰」。刚更新插件的人要到 `chrome://extensions` 点 GeminiForge 的重新加载（↻），插件才会取得通知权限。Gemini 标签页在后台超过 5 分钟后，Chrome 会降低它的检查频率，通知可能晚约一分钟。

## 开机自动启动（可选）

- 双击 `autostart_on.bat`：之后开机会自动以最小化窗口启动服务器
- 双击 `autostart_off.bat`：取消

## Gemini 能用的工具

- **文件与程序**：cd、list_dir、read_file、read_files、write_file、edit_file、edit_files、search、find_files、run（PowerShell）、job_status、stop_job、start_process、open、剪贴板
- **备份与恢复**：list_backups、restore_backup（见下方「文件备份」）
- **操作电脑**：screenshot、zoom（放大看小字）、click、double_click、right_click、move、drag、type、key、scroll（支持 `window_id` 定位、`percent` 跳转与内层容器 UIA 自动滚动补救）
- **精准点击与读取**：list_windows、focus_window、ui_elements（列出按钮／链接与输入框的值及勾选状态）、read_window（一次读取窗口或网页主内容的文字与字段值，含滚动区下方）、click_element（用名称或编号点击，屏幕外元素会自动滚进视区，并回报窗口文字）、type_element（一次完成对输入框聚焦、清空、输入与回读验证）

`ui_elements` 会返回 `window_id`，后续操作带上同一个 ID，能避免选到同名的浏览器标签页。已观察到的一串按钮可用 `click_elements {window_id, names}` 依次运行（最多 16 个）；任一步失败即停止。MCP 模式仍会由 Gemini 网页确认这一整串操作，并不会取消网页端的确认。窗口或元素失效时，工具会报错并要求重新观察，不会沿用旧坐标点击。

简中工具说明在 `extension/prompts/zh-CN.md`，其他语言在 `extension/prompts/`，可以自行修改。

## 文件备份

Gemini 每次用 `write_file` 或 `edit_file` 改文件前，会先把原文件复制到 `GeminiForge/backups/`，并在结果里附上备份编号。

- **请 Gemini 恢复**：跟它说「把刚刚改的文件恢复」，它会用 `list_backups` 找编号、`restore_backup` 恢复。
- **自己恢复**：双击 `restore.bat`，会列出最近 30 条备份，输入编号按 Enter 就好。
- 原本不存在、由 Gemini 新建的文件，恢复会把它删掉。
- 恢复前也会先备份当前的内容，所以恢复错了可以再恢复回来。
- 保留 30 天、最多 5000 条；超过 50 MB 的文件只记录、不备份内容。
- **`run` 运行的命令改到的文件不会备份**（例如 PowerShell 删文件、`git checkout`）。

## 小提醒

- **你也在用电脑时**：click、type、key 会用真的鼠标键盘，可能和你抢操作。click_element 对大部分按钮是直接触发，不会干扰你。
- **多显示器**：截屏默认拍鼠标所在的显示器；点击元素或切换窗口后，会自动拍那个窗口所在的显示器。

## 安全须知

- **紧急停止鼠标操作**：把鼠标用力甩到屏幕任一角落，正在进行的操作会立刻中止。
- Gemini 会读到它看过的网页和文件内容，这些内容可能藏有恶意指示。用「全部自动运行」时要特别小心。
- 服务器只接受本机上 Chrome 插件的请求，普通网页无法调用它。
- 你读取的文件和截屏内容会传到 Google 的 Gemini。

## 文件夹结构

```
GeminiForge/
├─ start.bat              启动插件用的服务器（方式一）
├─ start_mcp.bat          启动 MCP 服务器（方式二）
├─ autostart_on.bat       设置开机自动启动
├─ autostart_off.bat      取消开机自动启动
├─ restore.bat            列出备份并恢复文件
├─ *.sh                   macOS 和 Linux 用的同名脚本（start.sh 等）
├─ backups/               自动备份（不进 git）
├─ server/               本机服务器（Rust，插件和 MCP 共用同一个程序）
│  ├─ src/                所有工具、插件用的服务器、MCP 服务器、确认窗口、备份
│  ├─ tests/
│  ├─ Cargo.toml
│  ├─ exe.cmd             找到服务器程序，首次运行时自动构建
│  ├─ lang.cmd            批处理文件共用的多语言消息
│  └─ README.md           构建、环境变量、快速等待等细节
└─ extension/             Chrome 插件
   ├─ manifest.json
   ├─ background.js       把请求转给本机服务器
   ├─ content.js          在 Gemini 网页上：面板、抓工具指令、回传结果
   ├─ languages.js        语言标签对应表
   ├─ _locales/           多语言界面消息（en、zh_TW、zh_CN、ja、ko）
   └─ prompts/            给 Gemini 的多语言工具说明（如 zh-CN.md，可自行修改）
```

## 常见问题

- **圆点是红色**：`start.bat` 没开或被关掉了。
- **端口 18765 被占用**：在 `start.bat` 的 `call "%~dp0server\exe.cmd"` 上一行加 `set GF_PORT=其他端口号`，并把 `extension/manifest.json`、`extension/background.js` 里的 18765 一起改掉，再到 `chrome://extensions` 重新加载插件。
- **改了插件代码（content.js 等）**：到 `chrome://extensions` 点 GeminiForge 卡片上的重新加载 ↻，再刷新 Gemini。
- **Gemini 改版后插件失效**：Gemini 网页结构变了，需要更新 `content.js` 里找输入框、发送按钮的部分。
- **@GeminiForge 能不能完全自动、不用点确认？** 只能做到一部分：
  - **电脑上的确认窗口**：可以关。把 `server/mcp_config.json` 的 `approval` 改成 `none` 再重启服务器。
  - **Gemini 网页上的「允许 / 拒绝」**：关不掉。这是 Google 的安全机制，会改东西的工具（开程序、点击、改文件）每次都会问。
  - 可以把所有工具报告成「只会读取」来跳过网页确认：在 `server/mcp_config.json` 设 `"web_confirm": "off"` 再重启服务器（默认 `on`）。**风险自负**：这等于绕过 Google 的安全设计；Gemini 读到的网页或文件里可能藏有恶意指令（例如「删掉某文件夹」），没有确认就会直接在电脑上运行。建议只在你看着的时候使用。
  - 最接近全自动的做法：用 @GeminiForge 并把 `approval` 设为 `none`，每个会改东西的步骤在 Gemini 网页点一次「允许」；或改用 Chrome 插件（对话模式）的「全部自动运行」，完全不用点，但最好只在你看着的时候用。
