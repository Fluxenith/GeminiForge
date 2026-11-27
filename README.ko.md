[English](README.md) | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | **한국어**

# GeminiForge

웹 버전 Gemini(gemini.google.com)가 내 컴퓨터를 조작할 수 있게 해 줍니다. 파일 읽기와 쓰기, 명령 실행, 코드 작성, 화면 확인, 마우스와 키보드 제어가 가능합니다.
웹 버전 Gemini의 사용 한도를 쓰므로 API 키가 필요 없습니다.

```
gemini.google.com  ──(Gemini가 도구 명령 출력)──▶  Chrome 확장 프로그램  ──▶  로컬 서버 127.0.0.1:18765
        ▲                                                              │ 파일 / PowerShell / 스크린샷 / 마우스와 키보드
        └──────────(결과와 스크린샷을 채팅에 붙여 넣고 전송)◀───────────┘
```

## 다운로드

[Releases](../../releases/latest)에서 `GeminiForge-<버전>-windows-x64.zip`(Windows)을 내려받아 원하는 폴더에 압축을 풉니다. 미리 빌드된 서버(`server\geminiforge-server.exe`)가 들어 있어 따로 설치할 것이 없습니다. 처음 실행할 때 Windows SmartScreen 경고가 나오면 "추가 정보" → "실행"을 누릅니다.

소스에서 직접 빌드하려면 [Rust](https://rustup.rs)를 설치하세요. `start.bat`가 처음 실행될 때 자동으로 빌드합니다(1분 정도).

Mac이나 Linux 컴퓨터는 다음 절을 보세요.

## macOS와 Linux

Chrome 확장 프로그램(방법 1), MCP(방법 2), 파일과 명령 도구는 macOS와 Linux에서도 쓸 수 있습니다. Gemini는 `screenshot`으로 화면을 볼 수 있지만, 마우스·키보드 조작은 지금은 Windows에서만 됩니다. GeminiForge가 Gemini에게 자동으로 알려 줍니다.

1. [Releases](../../releases/latest)에서 컴퓨터에 맞는 zip을 내려받아 압축을 풉니다: `macos-arm64`(Apple 실리콘), `macos-x64`(Intel Mac), `linux-x64` 또는 `linux-arm64`.
2. `.bat` 파일 대신 터미널에서 `.sh` 스크립트를 실행합니다:
   ```
   cd GeminiForge
   sh start.sh            # 방법 1: 확장 프로그램용 서버
   sh start_mcp.sh        # 방법 2: MCP 서버
   sh restore.sh          # 백업 복원
   sh autostart_on.sh     # 로그인할 때 자동 시작(autostart_off.sh로 해제)
   ```
   사용하는 동안 터미널을 열어 두세요. 방법 1과 방법 2의 나머지 단계는 같습니다.

- `run`은 PowerShell 대신 bash를 사용합니다.
- **macOS**: 처음 실행할 때 "개발자를 확인할 수 없기 때문에 열 수 없습니다"가 나올 수 있습니다. "시스템 설정" → "개인정보 보호 및 보안"에서 geminiforge-server의 "그래도 허용"을 누르고 다시 실행하세요.
- **Linux**: MCP 확인 창에는 `zenity`(또는 `kdialog`), 클립보드 도구에는 `wl-clipboard`(Wayland)나 `xclip`(X11)이 필요합니다. 예: `sudo apt install zenity wl-clipboard xclip`
- **스크린샷**: macOS에서는 GeminiForge를 시작하는 터미널 앱에 화면 기록을 허용하세요("시스템 설정" → "개인정보 보호 및 보안" → "화면 기록"). Linux에서는 `gnome-screenshot`, `grim`, `spectacle`, `scrot`, ImageMagick 중 하나가 설치되어 있어야 합니다.
- 방법 2의 Tailscale 터널: `tailscale funnel --bg --https=10000 http://127.0.0.1:18767` (Mac App Store 버전의 명령은 `/Applications/Tailscale.app/Contents/MacOS/Tailscale`).
- 소스에서 빌드하려면 [Rust](https://rustup.rs)를 설치하세요. `start.sh`가 처음 실행될 때 자동으로 빌드합니다.

## 방법 1: Chrome 확장 프로그램

### 처음 설정

1. **로컬 서버 시작**: `start.bat`를 더블클릭합니다. 검은 창은 열어 두세요. 닫으면 서버가 멈춥니다.
2. **Chrome 확장 프로그램 설치**:
   1. Chrome 주소창에 `chrome://extensions`를 입력합니다
   2. 오른쪽 위의 "개발자 모드"를 켭니다
   3. "압축해제된 확장 프로그램을 로드합니다"를 클릭하고 이 폴더 안의 `extension` 폴더를 선택합니다
3. **Gemini 열기**: gemini.google.com으로 이동합니다(이미 열려 있다면 새로고침하세요). 오른쪽 아래에 **⚒ GeminiForge** 패널이 나타나고, 점이 초록색이면 서버에 연결된 것입니다.
4. **첫 스크린샷 업로드**: 패널의 "스크린샷 테스트"를 클릭합니다. Gemini가 이미지와 파일로 콘텐츠를 만드는 것에 대한 동의 창을 띄우면 동의하세요(한 번만 하면 됩니다). 테스트 스크린샷이 입력란에 들어가면 첨부 파일의 ✕를 눌러 제거합니다.

### 사용 방법

Gemini에서 **새 채팅**을 열고 원하는 작업을 입력해 전송합니다. 예: "바탕화면에 hello.py를 만들고 실행해 줘".
확장 프로그램이 첫 메시지 앞에 도구 안내를 자동으로 추가합니다(패널의 "새 채팅에 도구 안내 추가"가 선택되어 있을 때).

그 뒤로는 Gemini 답변에 들어 있는 도구 명령을 확장 프로그램이 찾아내 실행 모드에 따라 실행하고(또는 "실행"을 누를 때까지 기다리고), 결과를 자동으로 Gemini에 돌려보냅니다. 작업이 끝날 때까지 이 과정을 반복합니다.

이미 시작한 채팅에서 쓰려면 패널의 "도구 안내 삽입"을 클릭하세요.

오른쪽 아래의 실행 모드는 확장 프로그램의 `gf_tool` JSON 명령만 제어합니다. 화면 가운데에 "Let Gemini use … from GeminiForge"와 "허용 / 거절"이 나타나면 그 차례는 MCP 커넥터를 쓰고 있는 것입니다. 확장 프로그램의 "모두 자동 실행"으로는 이 확인이 없어지지 않습니다. 확장 프로그램 모드를 쓰려면 새 채팅을 만들고 "채팅" 모드를 선택한 뒤, `@GeminiForge`를 붙이지 말고 "새 채팅에 도구 안내 추가"를 선택된 상태로 두세요.

**Gem으로도 됩니다**: Gemini 사이드바의 Gem 페이지에서 새 Gem을 만들고 `extension/prompts/ko.md`(또는 다른 언어의 `extension/prompts/<언어>.md`) 내용을 "요청 사항"에 붙여 넣습니다. 그 Gem 안의 채팅에서는 확장 프로그램이 안내를 다시 추가하지 않습니다.

**Pro 또는 사고 모델을 권장합니다**: 여러 단계로 된 작업(코드 작성, 컴퓨터 조작)에서는 Pro가 Flash보다 훨씬 안정적입니다.

## 방법 2: @ 맞춤 앱(MCP, 권장)

Gemini 공식 도구 연결 방식입니다. Gemini 입력란에 `@GeminiForge`를 입력하면 바로 쓸 수 있습니다. Chrome 확장 프로그램이 필요 없고, Gemini 페이지가 바뀌어도 영향을 받지 않으며, 마찬가지로 웹 버전 사용 한도를 씁니다.

```
gemini.google.com ──(MCP, OAuth 인증)──▶ https://<내 PC>.ts.net:10000 ──Tailscale Funnel──▶ 127.0.0.1:18767 (geminiforge-server.exe mcp)
```

### 처음 설정

1. **MCP 서버 시작**: `start_mcp.bat`를 더블클릭합니다. 창에 **MCP 주소**와 **페어링 코드**가 표시됩니다. 열어 두세요.
2. **공개 터널 열기**(한 번만): PowerShell에서 실행합니다
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --bg --https=10000 http://127.0.0.1:18767
   ```
   Funnel이 이 컴퓨터의 공개 주소를 표시합니다. 그 주소 뒤에 `/mcp`를 붙이면 커넥터 주소가 됩니다. 예: `https://<기기 이름>.<tailnet>.ts.net:10000/mcp`. 자신의 주소를 사용하세요. 터널을 닫으려면:
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --https=10000 off
   ```
   Tailscale이 없나요? [cloudflared](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/)를 설치하고 `server/mcp_config.json`의 `"tunnel"`을 `"cloudflared"`로 바꾸세요. MCP 서버가 직접 빠른 터널을 열고 주소를 보여 줍니다. 이 주소는 서버를 시작할 때마다 바뀌므로, 시작할 때마다 새 주소를 Gemini의 맞춤 앱에 넣고 다시 페어링하세요.
3. **Gemini에 추가**: Gemini 입력란에 `@` 입력 → 모든 커넥터 보기 → 맞춤 앱 만들기 → MCP 주소 붙여넣기 → 계속.
   - "기타 설정"의 클라이언트 ID / 보안 비밀은 우선 비워 둬도 됩니다. Gemini가 입력을 요구하면 `server/mcp_config.json`의 `client_id`와 `client_secret`을 입력하세요.
4. **GeminiForge 연결** 페이지가 뜨면 서버 창에 표시된 **페어링 코드**를 입력하고 "연결 허용"을 누릅니다.

### 사용

1. Gemini를 엽니다. 둘 다 됩니다:
   - **Gemini 데스크톱 앱**(Windows): 영어가 아닌 UI의 "채팅" 모드에서도 바로 쓸 수 있습니다. `@` 메뉴에 GeminiForge가 바로 나오며, 영어 UI나 "작업" 모드로 바꿀 필요가 없습니다(2026-11-23, 앱 1.21.1에서 확인). 데스크톱 앱에서는 이 방법만 쓸 수 있고, 방법 1의 Chrome 확장 프로그램은 데스크톱 앱에 로드할 수 없습니다.
   - **웹 버전**: **영어 UI**인 `https://gemini.google.com/app?hl=en`을 사용합니다. 영어 UI에서는 일반 채팅(Chat)에서 @GeminiForge를 쓸 수 있고, @를 붙이지 않아도 자동으로 호출됩니다.
     영어가 아닌 UI의 "채팅" 모드에서는 @ 메뉴에 맞춤 앱이 나타나지 않아 "**작업**" 모드로 바꿔야 합니다(2026-11 확인).
2. `@`를 입력하고 메뉴에서 **GeminiForge**를 고른 뒤 원하는 작업을 입력합니다. 예: "내 바탕화면에 있는 파일을 나열해 줘".

무언가를 바꾸는 동작은 **Gemini 페이지가 먼저** "Let Gemini use … from GeminiForge"라고 묻습니다. "허용"을 눌러야 실행됩니다.

이어서 서버가 컴퓨터에 **두 번째 승인 창**을 띄웁니다(`approval`이 기본값 `readonly`일 때):
- **허용**: 이번 한 번만 허용
- **이 도구를 30분 동안 허용**: 같은 종류의 동작은 다시 묻지 않습니다
- **거절**, 또는 45초 동안 누르지 않으면 자동 거절(Gemini는 1분쯤 기다리다 포기합니다)

승인 모드는 `server/mcp_config.json`의 `approval`을 바꾸고 서버를 다시 시작하면 적용됩니다: `readonly`(기본값: Gemini 페이지 확인 + 컴퓨터 승인 창), `all`(읽기 전용 동작에도 창 표시), `none`(Gemini 페이지 확인만 사용하고 컴퓨터 창은 띄우지 않음).

### 보안

- 연결하려면 페어링 코드를 입력해야 합니다. 5번 틀리면 10분 동안 잠깁니다.
- `server/mcp_config.json`(페어링 코드, 보안 비밀)과 `server/mcp_auth.json`(로그인 자격 증명)은 다른 사람에게 공유하지 마세요.
- 연결된 기기를 모두 무효화하려면 서버를 끄고 `server/mcp_auth.json`을 삭제한 뒤 다시 시작합니다. 페어링 코드를 바꾸려면 `mcp_config.json`에서 `pairing_code` 줄을 삭제하고 다시 시작합니다.
- 터널을 열면 누구나 이 주소에 접속할 수 있지만, 페어링 코드가 없으면 어떤 도구도 쓸 수 없습니다.

## 패널 설명

Gemini 페이지 오른쪽 아래의 **⚒ GeminiForge**를 펼치고 "언어" 메뉴에서 **English, 繁體中文, 简体中文, 日本語, 한국어**를 전환합니다. 처음 열 때는 Chrome의 UI 언어를 따르고, 그 밖의 언어에서는 영어로 표시됩니다. 선택은 현재 Chrome 프로필에 저장되며 열려 있는 Gemini 탭에도 동기화됩니다.

패널, 확인 버튼, 이후에 삽입하는 도구 안내가 모두 선택한 언어로 표시됩니다. 로컬 서버가 연결되면 다음에 열리는 MCP 페어링 페이지와 데스크톱 승인 창에도 적용됩니다. 기존 설치를 업데이트할 때는 확장 프로그램을 다시 로드하고 Gemini 탭을 새로고침한 뒤 `start.bat`의 로컬 서버를 다시 시작하세요. MCP도 쓰고 있다면 `start_mcp.bat`도 함께 다시 시작하세요.

| 항목 | 역할 |
|---|---|
| 점 | 초록색: 서버 연결됨. 빨간색: `start.bat`를 실행하세요 |
| 사용 | 끄면 확장 프로그램이 어떤 도구도 실행하지 않습니다 |
| 새 채팅에 도구 안내 추가 | 새 채팅에서 첫 메시지를 보낼 때 도구 안내를 앞에 자동으로 붙입니다 |
| 답변이 끝나면 알림 | Gemini 답변이 끝났을 때(또는 도구 작업 전체가 끝났을 때), "실행"을 누르기를 기다릴 때, 오류가 났을 때 Windows 알림을 띄웁니다. 알림을 클릭하면 해당 Gemini 탭으로 돌아갑니다. 기본값 "탭을 떠나 있을 때만"은 다른 탭이나 앱을 보고 있을 때만 알리고, Gemini 탭을 보고 있을 때는 알리지 않습니다. "항상 알림"은 탭을 보고 있어도 알리고, "끄기"는 전혀 알리지 않습니다. 옆의 "알림 테스트"는 알림을 바로 하나 보내 Windows가 막고 있지 않은지 확인할 수 있습니다. 알림을 보낼 때마다 패널 기록에 🔔 줄이 하나 추가됩니다. "사용"도 선택되어 있어야 합니다 |
| 실행 모드 | **단계마다 확인**: 가장 안전<br>**읽기 전용 도구 외에는 확인**(기본값): 파일 보기와 스크린샷은 자동 실행. 파일 쓰기, 명령 실행, 클릭은 "실행"을 눌러야 합니다<br>**모두 자동 실행**: 완전히 맡깁니다. 신뢰하는 작업에만 쓰세요 |
| 중지 | 진행 중인 작업을 즉시 중단합니다 |
| 확인 상자의 "이 채팅에서 허용" | 같은 채팅에서는 이 도구를 다시 묻지 않습니다(채팅을 바꾸면 초기화) |
| 스크린샷 테스트 | 스크린샷과 업로드가 정상인지 확인합니다 |

자동 실행이 200회 이어지면 멈추고 계속할지 묻습니다.

알림이 보이지 않을 때: Windows "설정 → 시스템 → 알림"에서 **Google Chrome** 알림이 켜져 있는지 확인하고 "방해 금지"를 끕니다. 확장 프로그램을 방금 업데이트했다면 `chrome://extensions`에서 GeminiForge의 새로고침(↻)을 눌러야 확장 프로그램이 알림 권한을 받습니다. Gemini 탭이 5분 넘게 백그라운드에 있으면 Chrome이 확인 빈도를 낮추므로 알림이 1분쯤 늦을 수 있습니다.

## Windows 시작 시 자동 실행(선택)

- `autostart_on.bat` 더블클릭: 이후 부팅할 때 서버가 최소화된 창으로 자동 시작됩니다
- `autostart_off.bat` 더블클릭: 해제

## Gemini가 쓸 수 있는 도구

- **파일과 프로그램**: cd, list_dir, read_file, read_files, write_file, edit_file, edit_files, search, find_files, run(PowerShell), job_status, stop_job, start_process, open, 클립보드
- **백업과 복원**: list_backups, restore_backup(아래 "파일 백업" 참고)
- **컴퓨터 조작**: screenshot, zoom(작은 글씨 확대), click, double_click, right_click, move, drag, type, key, scroll(`window_id` 지정, `percent` 이동, 안쪽 컨테이너의 UIA 자동 스크롤 보완 지원)
- **정확한 클릭과 읽기**: list_windows, focus_window, ui_elements(버튼 / 링크와 입력란의 값 및 선택 상태 나열), read_window(창이나 웹 페이지 주요 내용의 텍스트와 필드 값을 스크롤 영역 아래까지 한 번에 읽기), click_element(이름이나 번호로 클릭. 화면 밖 요소는 자동으로 보이는 영역으로 스크롤하고 창 텍스트를 보고), type_element(입력란 포커스, 지우기, 입력, 다시 읽어 확인까지 한 번에 수행)

`ui_elements`는 `window_id`를 반환합니다. 이후 동작에 같은 ID를 넘기면 이름이 같은 브라우저 탭을 잘못 고르는 일을 막을 수 있습니다. 이미 확인한 일련의 버튼은 `click_elements {window_id, names}`로 차례대로 실행할 수 있습니다(최대 16개). 하나라도 실패하면 거기서 멈춥니다. MCP 모드에서는 이 일련의 동작 전체를 Gemini 페이지가 확인하며, 페이지 쪽 확인이 없어지는 것은 아닙니다. 창이나 요소가 더 이상 유효하지 않으면 도구는 오류를 보고하고 다시 확인하라고 요청하며, 이전 좌표로 클릭하지 않습니다.

한국어 도구 안내는 `extension/prompts/ko.md`에, 다른 언어는 `extension/prompts/`에 있으며 직접 수정할 수 있습니다.

## 파일 백업

Gemini가 `write_file`이나 `edit_file`로 파일을 바꿀 때마다 먼저 원본을 `GeminiForge/backups/`에 복사하고, 결과에 백업 번호를 붙입니다.

- **Gemini에게 복원 요청**: "방금 바꾼 파일을 복원해 줘"라고 말하면 `list_backups`로 번호를 찾고 `restore_backup`으로 복원합니다.
- **직접 복원**: `restore.bat`를 더블클릭하면 최근 백업 30개가 표시됩니다. 번호를 입력하고 Enter를 누르면 됩니다.
- 원래 없었고 Gemini가 새로 만든 파일은 복원하면 삭제됩니다.
- 복원하기 전에도 현재 내용을 먼저 백업하므로, 잘못 복원해도 다시 되돌릴 수 있습니다.
- 30일 동안, 최대 5000개를 보관합니다. 50 MB를 넘는 파일은 기록만 하고 내용은 백업하지 않습니다.
- **`run`으로 실행한 명령이 바꾼 파일은 백업되지 않습니다**(예: PowerShell로 파일 삭제, `git checkout`).

## 참고 사항

- **나도 컴퓨터를 쓰고 있을 때**: click, type, key는 실제 마우스와 키보드를 쓰므로 조작이 겹칠 수 있습니다. click_element는 대부분의 버튼을 직접 작동시키므로 방해가 되지 않습니다.
- **다중 모니터**: 스크린샷은 기본적으로 마우스가 있는 모니터를 찍습니다. 요소를 클릭하거나 창을 전환한 뒤에는 그 창이 있는 모니터를 자동으로 찍습니다.

## 안전 유의 사항

- **마우스 조작 긴급 정지**: 마우스를 화면의 아무 모서리로 세게 밀면 진행 중인 조작이 즉시 중단됩니다.
- Gemini는 자신이 본 웹 페이지와 파일의 내용을 읽으며, 그 안에 악의적인 지시가 숨어 있을 수 있습니다. "모두 자동 실행"을 쓸 때는 특히 주의하세요.
- 서버는 이 컴퓨터의 Chrome 확장 프로그램이 보낸 요청만 받습니다. 일반 웹 페이지는 서버를 호출할 수 없습니다.
- 읽은 파일과 스크린샷 내용은 Google의 Gemini로 전송됩니다.

## 폴더 구조

```
GeminiForge/
├─ start.bat              확장 프로그램용 서버 시작(방법 1)
├─ start_mcp.bat          MCP 서버 시작(방법 2)
├─ autostart_on.bat       Windows 시작 시 자동 실행 설정
├─ autostart_off.bat      Windows 시작 시 자동 실행 해제
├─ restore.bat            백업을 나열하고 파일 복원
├─ *.sh                   macOS와 Linux용 같은 이름의 스크립트(start.sh 등)
├─ backups/               자동 백업(git에 포함하지 않음)
├─ server/               로컬 서버(Rust, 확장 프로그램과 MCP가 같은 프로그램 사용)
│  ├─ src/                모든 도구, 확장 프로그램용 서버, MCP 서버, 승인 창, 백업
│  ├─ tests/
│  ├─ Cargo.toml
│  ├─ exe.cmd             서버 프로그램을 찾고, 처음 실행할 때 자동 빌드
│  ├─ lang.cmd            배치 파일 공용 다국어 메시지
│  └─ README.md           빌드, 환경 변수, 빠른 대기 등 세부 내용
└─ extension/             Chrome 확장 프로그램
   ├─ manifest.json
   ├─ background.js       요청을 로컬 서버로 전달
   ├─ content.js          Gemini 페이지에서: 패널, 도구 명령 감지, 결과 반환
   ├─ languages.js        언어 태그 대응표
   ├─ _locales/           다국어 UI 메시지(en, zh_TW, zh_CN, ja, ko)
   └─ prompts/            Gemini용 다국어 도구 안내(ko.md 등. 수정 가능)
```

## 자주 묻는 질문

- **점이 빨간색입니다**: `start.bat`가 실행되지 않았거나 닫혔습니다.
- **포트 18765가 사용 중입니다**: `start.bat`의 `call "%~dp0server\exe.cmd"` 앞 줄에 `set GF_PORT=다른 포트 번호`를 추가하고, `extension/manifest.json`과 `extension/background.js`의 18765도 함께 바꾼 뒤 `chrome://extensions`에서 확장 프로그램을 다시 로드합니다.
- **확장 프로그램 코드(content.js 등)를 바꿨습니다**: `chrome://extensions`에서 GeminiForge 카드의 새로고침 ↻을 누르고 Gemini를 새로고침합니다.
- **Gemini 업데이트 후 확장 프로그램이 작동하지 않습니다**: Gemini 페이지 구조가 바뀐 것입니다. `content.js`에서 입력란과 전송 버튼을 찾는 부분을 업데이트해야 합니다.
- **@GeminiForge를 확인 없이 완전히 자동으로 쓸 수 있나요?** 일부만 가능합니다:
  - **컴퓨터의 승인 창**: 끌 수 있습니다. `server/mcp_config.json`의 `approval`을 `none`으로 바꾸고 서버를 다시 시작하세요.
  - **Gemini 페이지의 "허용 / 거절"**: 끌 수 없습니다. Google의 안전 장치이며, 무언가를 바꾸는 도구(프로그램 실행, 클릭, 파일 변경)는 매번 묻습니다.
  - 모든 도구를 "읽기 전용"으로 보고하게 해서 페이지 확인을 건너뛸 수 있습니다: `server/mcp_config.json`에 `"web_confirm": "off"`를 설정하고 서버를 다시 시작합니다(기본값 `on`). **위험은 본인 책임입니다**: 이는 Google의 안전 설계를 우회하는 것입니다. Gemini가 읽은 웹 페이지나 파일에 악의적인 명령(예: "이 폴더를 삭제해")이 숨어 있으면 확인 없이 바로 컴퓨터에서 실행됩니다. 지켜보고 있을 때만 쓰기를 권합니다.
  - 완전 자동에 가장 가까운 방법: @GeminiForge를 쓰면서 `approval`을 `none`으로 두고, 무언가를 바꾸는 단계마다 Gemini 페이지에서 "허용"을 한 번씩 누릅니다. 또는 Chrome 확장 프로그램(채팅 모드)의 "모두 자동 실행"을 씁니다. 이쪽은 아무것도 누를 필요가 없지만, 지켜보고 있을 때만 쓰는 것이 좋습니다.
