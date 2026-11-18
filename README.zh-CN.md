# GeminiForge

[English](README.md) | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md)

Turn Gemini Web into a local computer agent — files, terminal, and desktop control.

让网页版 Gemini（gemini.google.com）能操作你的电脑：读写文件、运行命令、写程序、看屏幕并控制鼠标键盘。
使用的是你网页版 Gemini 的额度，不需要 API key。

## 环境要求

- Windows 10 / 11
- Python 3.10 或以上版本，安装时勾选“Add Python to PATH”
- 可以登录使用的 Gemini 账号
- Chrome 扩展程序模式需要 Google Chrome；MCP 模式需要安装并登录 Tailscale，且账号可以使用 Funnel

下载本项目后，将整个文件夹解压到固定位置，再按下方任一方式设置。

扩展程序面板、桌面确认窗口、服务器窗口和 .bat 脚本都会使用扩展程序面板中选择的语言（默认为 Windows 显示语言）。

```
gemini.google.com  ──(Gemini 输出工具命令)──▶  Chrome 扩展程序  ──▶  本地服务器 127.0.0.1:18765
        ▲                                                              │ 读写文件 / PowerShell / 截屏 / 鼠标键盘
        └──────────────(结果和截图自动贴回对话并发送)◀──────────────────┘
```

## 方式一：Chrome 扩展程序

### 首次设置

1. **启动本地服务器**：双击 `start.bat`。首次运行会自动安装依赖包。黑色窗口要保持打开，关掉就会停止。
2. **安装 Chrome 扩展程序**：
   1. 在 Chrome 地址栏输入 `chrome://extensions`
   2. 打开右上角的“开发者模式”
   3. 点击“加载已解压的扩展程序”，选择本文件夹里的 `extension` 文件夹
3. **打开 Gemini**：访问 gemini.google.com（已经打开的请刷新），右下角会出现 **⚒ GeminiForge** 面板，圆点变绿表示已连上服务器。
4. **首次上传截图**：点击面板上的“测试截屏”。如果 Gemini 弹出关于根据图像和文件生成内容的窗口，点击“同意”（只需一次）。测试截图放进输入框后，点击附件上的 ✕ 移除即可。

### 使用方法

在 Gemini 新建一个**新对话**，直接输入你的需求并发送，例如“帮我在桌面创建一个 hello.py 并运行它”。
扩展程序会自动在第一条消息前面加上工具说明（面板上“新对话自动添加工具说明”勾选时）。

之后 Gemini 回复里的工具命令会被扩展程序捕获，按照模式运行（或等你点击“运行”），结果自动发回给 Gemini，一直重复到任务完成。

想在已经开始的对话中使用，可以点击面板上的“插入工具说明”。

右下角的运行模式只控制扩展程序的 `gf_tool` JSON 命令。如果屏幕中央出现“Let Gemini use … from GeminiForge”以及“Allow / Deny”，说明这一轮使用的是 MCP 连接器；扩展程序的“全部自动运行”不会取消这层确认。要使用扩展程序模式，请新建对话、选择对话（Chat）模式，不要标记 `@GeminiForge`，并保持“新对话自动添加工具说明”勾选。

**也可以用 Gem**：在 Gemini 左侧的 Gem manager 中新建一个 Gem，把 `extension/prompts/zh-CN.md` 的内容粘贴到 Instructions 里。在该 Gem 中的对话，扩展程序不会再重复添加说明。

**建议使用 Pro 或“思考型”模型**：多步骤任务（写程序、操作电脑）Pro 比 Flash 稳定得多。

## 方式二：@ 自定义应用（MCP，推荐）

通过 Gemini 的自定义应用连接 MCP，在输入框中选择 `@GeminiForge` 即可使用。不需要 Chrome 扩展程序，同样使用网页版额度。自定义应用是否可用，取决于 Gemini 账号、界面语言和模式。

```
gemini.google.com ──(MCP，OAuth 验证)──▶ https://<你的电脑>.ts.net:10000 ──Tailscale Funnel──▶ 127.0.0.1:18766 (mcp_server.py)
```

### 首次设置

1. **启动 MCP 服务器**：双击 `start_mcp.bat`。窗口会显示 **MCP 网址** 和 **配对码**，要保持打开。
2. **开启公开通道**（只需一次）：在 PowerShell 中运行
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --bg --https=10000 http://127.0.0.1:18766
   ```
   Funnel 会显示这台电脑的公开网址，在网址后面加上 `/mcp` 就是连接器网址，例如 `https://<设备名称>.<tailnet>.ts.net:10000/mcp`。请使用你自己的网址。要关闭通道：
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --https=10000 off
   ```
3. **在 Gemini 中添加**：在 Gemini 输入框输入 `@` → 查看所有连接器 → 创建自定义应用 → 粘贴 MCP 网址 → 继续。
   - 其他设置中的客户端 ID／密钥可以先留空。如果 Gemini 要求填写，请填入 `server/mcp_config.json` 里的 `client_id` 和 `client_secret`。
4. 弹出 **连接 GeminiForge** 页面时，输入服务器窗口上的 **配对码**，点击“允许连接”。

### 使用

1. 用**英文界面**打开 Gemini：`https://gemini.google.com/app?hl=en`（或使用 Gemini 桌面应用）。英文界面的普通对话（Chat）就能使用 @GeminiForge，甚至不输入 @ 也会自动调用。
   中文界面的“对话”模式中，@ 菜单里不会出现自定义应用，只能切换到“**任务**”模式使用（2026-11 实测）。
2. 输入 `@`，从菜单中选择 **GeminiForge**，再输入你的需求，例如“列出我桌面上的文件”。

对于会改变内容的操作，**Gemini 网页会先询问一次**“Let Gemini use … from GeminiForge”，点击“Allow”后才会执行。

接着服务器还会在你的电脑上弹出**第二道确认窗口**（`approval` 为默认的 `readonly` 时）：
- **允许**：只允许这一次
- **30 分钟内都允许这个工具**：之后同类操作不再询问
- **拒绝**，或 45 秒内未点击则自动拒绝（Gemini 大约等一分钟就会放弃）

确认模式可以在 `server/mcp_config.json` 的 `approval` 中修改，然后重启服务器：`readonly`（默认：Gemini 网页确认 + 电脑确认窗口）、`all`（连只读操作也弹窗）、`none`（只依靠 Gemini 网页的确认，不在电脑上弹窗）。

### 安全

- 必须输入配对码才能连接；连续错 5 次会锁定 10 分钟。
- 不要把 `server/mcp_config.json`（配对码、密钥）和 `server/mcp_auth.json`（登录凭据）分享给别人。
- 想让所有已连接的设备失效：关闭服务器，删除 `server/mcp_auth.json`，再重新启动。想更换配对码：删除 `mcp_config.json` 里 `pairing_code` 那一行再重启。
- 通道一旦打开，任何人都能访问这个网址，但没有配对码就无法使用任何工具。

## 面板说明

在 Gemini 页面右下角展开 **⚒ GeminiForge**，用“语言”菜单切换 **English、繁體中文、简体中文、日本語、한국어**。首次打开时按 Chrome 的界面语言显示，其他语言默认使用英文。选择会保存在当前的 Chrome 个人资料中，并同步到已打开的 Gemini 标签页。

面板、确认按钮以及之后插入的工具说明都会使用所选语言。本地服务器连接后，下一次打开的 MCP 配对页面和桌面确认窗口也会应用该设置。更新现有安装时，请重新加载扩展程序、刷新 Gemini 标签页，并重新启动 `start.bat` 的本地服务器。如果也在使用 MCP，请一并重新启动 `start_mcp.bat`。

| 项目 | 作用 |
|---|---|
| 圆点 | 绿色：服务器已连接；红色：请运行 `start.bat` |
| 启用 | 关闭后扩展程序不会运行任何工具 |
| 新对话自动添加工具说明 | 在新对话中发送第一条消息时，自动把工具说明加在前面 |
| 模式 | **每一步都要确认**：最安全<br>**只读工具自动运行，其余需确认**（默认）：查看文件、截屏自动运行；写文件、运行命令、点击鼠标需要你点击“运行”<br>**全部自动运行**：完全放手，请只在你信任的任务中使用 |
| 停止 | 立即中止当前流程 |
| 确认框中的“本对话都允许” | 在同一个对话里，这个工具之后不再询问（换对话后重置） |
| 测试截屏 | 检查截屏和上传是否正常 |

连续自动运行 40 轮后会停下来询问是否继续。

## 开机自动启动（可选）

- 双击 `autostart_on.bat`：之后开机会以最小化窗口自动启动服务器
- 双击 `autostart_off.bat`：取消

## Gemini 可用的工具

- **文件与程序**：list_dir、read_file、write_file、edit_file、search、find_files、run（PowerShell）、start_process、open、剪贴板
- **备份与恢复**：list_backups、restore_backup（见下方“文件备份”）
- **操作电脑**：screenshot、zoom（放大查看小字）、click、double_click、right_click、move、drag、type、key、scroll
- **精准点击**：list_windows、focus_window、ui_elements（通过 Windows 无障碍接口列出按钮和链接的名称与位置）、click_element（按名称或编号点击，直接触发按钮：不移动你的鼠标，窗口在后台也可以，并返回窗口显示的文字）

`ui_elements` 会返回 `window_id`，后续操作带上同一个 ID，可以避免选中同名的浏览器标签页。已观察到的一串按钮可以用 `click_elements {window_id, names}` 依次执行（最多 16 个）；任一步失败即停止。MCP 模式下 Gemini 网页仍会确认这一整串操作，并不会取消网页端的确认。窗口或元素失效时，工具会报错并要求重新观察，不会沿用旧坐标点击。

工具说明在 `extension/prompts/`，每种语言一个文件，可以自行修改。

## 文件备份

Gemini 每次用 `write_file` 或 `edit_file` 修改文件前，会先把原文件复制到 `GeminiForge/backups/`，并在结果中附上备份编号。

- **让 Gemini 恢复**：对它说“把刚才改的文件恢复”，它会用 `list_backups` 查找编号，用 `restore_backup` 恢复。
- **自己恢复**：双击 `restore.bat`，会列出最近 30 条备份，输入编号后按 Enter 即可。
- 原本不存在、由 Gemini 新建的文件，恢复时会被删除。
- 恢复前也会先备份当前内容，所以恢复错了还可以再恢复回来。
- 保留 30 天、最多 5000 条；超过 50 MB 的文件只记录、不备份内容。
- **`run` 运行的命令所修改的文件不会备份**（例如 PowerShell 删除文件、`git checkout`）。

## 小提示

- **你同时也在使用电脑时**：click、type、key 会使用真实的鼠标键盘，可能和你抢操作。click_element 对大部分按钮是直接触发，不会干扰你。
- **多屏幕**：截屏默认拍摄鼠标所在的屏幕；点击元素或切换窗口后，会自动拍摄该窗口所在的屏幕。

## 安全须知

- **紧急停止鼠标操作**：把鼠标用力甩到屏幕任一角落，正在进行的操作会立即中止。
- Gemini 会读到它看过的网页和文件内容，这些内容可能藏有恶意指令。使用“全部自动运行”时要特别小心。
- 服务器只接受本机 Chrome 扩展程序的请求，普通网页无法调用它。
- 你读取的文件和截图内容会发送到 Google 的 Gemini。

## 文件夹结构

```
GeminiForge/
├─ start.bat              启动扩展程序用的服务器（方式一）
├─ start_mcp.bat          启动 MCP 服务器（方式二）
├─ autostart_on.bat       设置开机自动启动
├─ autostart_off.bat      取消开机自动启动
├─ restore.bat            列出备份并恢复文件
├─ backups/               自动备份（不纳入 git）
├─ server/
│  ├─ server.py           所有工具的实现，也是扩展程序用的服务器
│  ├─ mcp_server.py       MCP 服务器（OAuth 验证 + 确认窗口）
│  ├─ approve_dialog.py   电脑上的确认窗口
│  ├─ backups.py          改文件前自动备份、恢复
│  └─ requirements.txt
└─ extension/             Chrome 扩展程序
   ├─ manifest.json
   ├─ background.js       把请求转发给本地服务器
   ├─ content.js          在 Gemini 网页上：面板、捕获工具命令、返回结果
   └─ prompts/            给 Gemini 的工具说明，每种语言一个文件（可自行修改）
      └─ zh-TW.md、zh-CN.md、en.md、ja.md、ko.md
```

## 常见问题

- **圆点是红色**：`start.bat` 没有运行或已被关闭。
- **端口 18765 被占用**：在 `start.bat` 的 `set PYTHONIOENCODING` 下一行加上 `set GF_PORT=其他端口号`，并把 `extension/manifest.json`、`extension/background.js` 里的 18765 一起改掉，再到 `chrome://extensions` 重新加载扩展程序。
- **修改了扩展程序代码（content.js 等）**：到 `chrome://extensions` 点击 GeminiForge 卡片上的重新加载 ↻，再刷新 Gemini。
- **Gemini 改版后扩展程序失效**：Gemini 网页结构变了，需要更新 `content.js` 中查找输入框、发送按钮的部分。
- **@GeminiForge 能不能完全自动、不用点确认？** 只能做到一部分：
  - **电脑上的确认窗口**：可以关闭。把 `server/mcp_config.json` 的 `approval` 改成 `none` 再重启服务器。
  - **Gemini 网页上的“Allow / Deny”**：无法关闭。这是 Google 的安全机制，会改变内容的工具（打开程序、点击、修改文件）每次都会询问。
  - 可以把所有工具报告为“只读”来跳过网页确认：在 `server/mcp_config.json` 中设置 `"web_confirm": "off"` 再重启服务器（默认 `on`）。**风险自负**：这等于绕过 Google 的安全设计；Gemini 读到的网页或文件里可能藏有恶意指令（例如“删除某个文件夹”），没有确认就会直接在电脑上执行。建议只在你看着的时候使用。
  - 最接近全自动的做法：使用 @GeminiForge 并把 `approval` 设为 `none`，每个会改变内容的步骤在 Gemini 网页点一次“Allow”；或改用 Chrome 扩展程序（对话模式）的“全部自动运行”，完全不用点击，但最好只在你看着的时候使用。

## 许可

由 [Fluxenith](https://github.com/Fluxenith) 维护。采用 MIT 许可，详见 [LICENSE](LICENSE)。本项目与 Google 无隶属或背书关系。
