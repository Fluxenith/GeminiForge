# GeminiForge

[English](README.md) | [繁體中文](README.zh-TW.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md)

Turn Gemini Web into a local computer agent — files, terminal, and desktop control.

웹 버전 Gemini(gemini.google.com)가 내 컴퓨터를 조작할 수 있게 해 줍니다. 파일 읽기/쓰기, 명령 실행, 코드 작성, 화면 보기, 마우스와 키보드 제어가 가능합니다.
웹 버전 Gemini의 사용량을 그대로 사용하므로 API 키가 필요 없습니다.

## 요구 사항

- Windows 10 / 11
- Python 3.10 이상 (설치할 때 "Add Python to PATH"에 체크)
- 로그인해서 사용할 수 있는 Gemini 계정
- Chrome 확장 프로그램 모드에는 Google Chrome이 필요하고, MCP 모드에는 Tailscale을 설치하고 로그인해야 하며 계정에서 Funnel을 사용할 수 있어야 합니다

프로젝트를 다운로드한 뒤 폴더 전체를 고정된 위치에 압축 해제하고, 아래 방법 중 하나로 설정하세요.

확장 프로그램 패널, 데스크톱 확인 팝업, 서버 창, .bat 스크립트는 확장 프로그램 패널에서 선택한 언어를 따릅니다(기본값은 Windows 표시 언어).

```
gemini.google.com  ──(Gemini가 도구 명령 출력)──▶  Chrome 확장 프로그램  ──▶  로컬 서버 127.0.0.1:18765
        ▲                                                              │ 파일 읽기/쓰기 / PowerShell / 스크린샷 / 마우스·키보드
        └──────────────(결과와 스크린샷을 대화에 자동으로 붙여 넣고 전송)◀──────────────────┘
```

## 방법 1: Chrome 확장 프로그램

### 처음 설정

1. **로컬 서버 시작**: `start.bat`을 더블클릭합니다. 처음 실행할 때 필요한 패키지가 자동으로 설치됩니다. 검은 창은 계속 열어 두세요. 닫으면 서버가 멈춥니다.
2. **Chrome 확장 프로그램 설치**:
   1. Chrome 주소창에 `chrome://extensions` 입력
   2. 오른쪽 위의 "개발자 모드" 켜기
   3. "압축해제된 확장 프로그램을 로드합니다"를 클릭하고 이 폴더 안의 `extension` 폴더 선택
3. **Gemini 열기**: gemini.google.com에 접속합니다(이미 열려 있으면 새로고침). 오른쪽 아래에 **⚒ GeminiForge** 패널이 나타나고, 동그라미가 초록색이면 서버에 연결된 것입니다.
4. **첫 스크린샷 업로드**: 패널의 "스크린샷 테스트"를 클릭합니다. Gemini에 이미지와 파일로 콘텐츠를 만드는 것에 대한 동의 창이 뜨면 동의하세요(한 번만 필요). 테스트 스크린샷이 입력창에 들어가면 첨부 파일의 ✕를 눌러 제거하면 됩니다.

### 사용 방법

Gemini에서 **새 채팅**을 열고 원하는 작업을 입력해 보내기만 하면 됩니다. 예: "바탕 화면에 hello.py를 만들고 실행해 줘".
확장 프로그램이 첫 메시지 앞에 도구 안내를 자동으로 붙입니다(패널의 "새 채팅에 도구 안내 추가"가 체크되어 있을 때).

그 뒤로는 Gemini 답변에 포함된 도구 명령을 확장 프로그램이 감지해 모드에 따라 실행하고(또는 "실행"을 누를 때까지 기다리고), 결과를 자동으로 Gemini에 돌려보냅니다. 작업이 끝날 때까지 이를 반복합니다.

이미 시작된 채팅에서 사용하려면 패널의 "도구 안내 삽입"을 클릭하세요.

오른쪽 아래의 실행 모드는 확장 프로그램의 `gf_tool` JSON 명령만 제어합니다. 화면 가운데에 "Let Gemini use … from GeminiForge"와 "Allow / Deny"가 나타나면 그 차례는 MCP 커넥터를 사용하는 것이며, 확장 프로그램의 "모두 자동 실행"으로는 이 확인이 사라지지 않습니다. 확장 프로그램 모드를 사용하려면 새 채팅을 만들고 Chat 모드를 선택한 뒤, `@GeminiForge`를 태그하지 말고 "새 채팅에 도구 안내 추가"를 체크한 상태로 두세요.

**Gem에서도 사용할 수 있습니다**: Gemini 왼쪽의 Gem manager에서 새 Gem을 만들고 `extension/prompts/ko.md`의 내용을 Instructions에 붙여 넣으세요. 그 Gem 안의 채팅에서는 확장 프로그램이 안내를 다시 붙이지 않습니다.

**Pro 또는 "사고형" 모델을 권장합니다**: 여러 단계로 이루어진 작업(코드 작성, 컴퓨터 조작)에서는 Pro가 Flash보다 훨씬 안정적입니다.

## 방법 2: @ 맞춤 앱 (MCP, 권장)

Gemini의 맞춤 앱으로 MCP에 연결한 뒤, 입력창에서 `@GeminiForge`를 선택하면 사용할 수 있습니다. Chrome 확장 프로그램이 필요 없으며, 마찬가지로 웹 버전 사용량을 씁니다. 맞춤 앱을 사용할 수 있는지는 Gemini 계정, 인터페이스 언어, 모드에 따라 다릅니다.

```
gemini.google.com ──(MCP, OAuth 인증)──▶ https://<내 컴퓨터>.ts.net:10000 ──Tailscale Funnel──▶ 127.0.0.1:18766 (mcp_server.py)
```

### 처음 설정

1. **MCP 서버 시작**: `start_mcp.bat`을 더블클릭합니다. 창에 **MCP URL**과 **페어링 코드**가 표시되며, 계속 열어 두어야 합니다.
2. **공개 터널 열기** (한 번만): PowerShell에서 실행
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --bg --https=10000 http://127.0.0.1:18766
   ```
   Funnel이 이 컴퓨터의 공개 URL을 보여 줍니다. URL 뒤에 `/mcp`를 붙이면 커넥터 URL이 됩니다. 예: `https://<기기 이름>.<tailnet>.ts.net:10000/mcp`. 반드시 본인의 URL을 사용하세요. 터널을 닫으려면:
   ```
   & "C:\Program Files\Tailscale\tailscale.exe" funnel --https=10000 off
   ```
3. **Gemini에 추가**: Gemini 입력창에 `@` 입력 → 모든 커넥터 보기 → 맞춤 앱 만들기 → MCP URL 붙여 넣기 → 계속.
   - 추가 설정의 클라이언트 ID/보안 비밀은 처음에는 비워 두어도 됩니다. Gemini가 입력을 요구하면 `server/mcp_config.json`의 `client_id`와 `client_secret`을 입력하세요.
4. **GeminiForge 연결** 페이지가 나타나면 서버 창에 표시된 **페어링 코드**를 입력하고 "연결 허용"을 클릭합니다.

### 사용

1. **영어 인터페이스**로 Gemini를 여세요: `https://gemini.google.com/app?hl=en` (또는 Gemini 데스크톱 앱). 영어 인터페이스에서는 일반 채팅(Chat)에서 @GeminiForge를 사용할 수 있고, @를 입력하지 않아도 자동으로 호출되기도 합니다.
   중국어 인터페이스의 "對話"(Chat) 모드에서는 @ 메뉴에 맞춤 앱이 나타나지 않아 "**任務**" 모드로 전환해야만 사용할 수 있습니다(2026-11 테스트 기준).
2. `@`를 입력하고 메뉴에서 **GeminiForge**를 선택한 다음 원하는 작업을 입력합니다. 예: "바탕 화면에 있는 파일 목록을 보여 줘".

무언가를 변경하는 작업은 **Gemini 웹 페이지가 먼저 한 번 묻습니다**("Let Gemini use … from GeminiForge"). "Allow"를 클릭해야 실행됩니다.

이어서 서버가 내 컴퓨터에 **두 번째 확인 창**을 띄웁니다(`approval`이 기본값 `readonly`일 때):
- **허용**: 이번 한 번만 허용
- **이 도구를 30분 동안 허용**: 이후 같은 종류의 작업은 다시 묻지 않음
- **거절**, 또는 45초 동안 누르지 않으면 자동으로 거절 (Gemini는 1분 정도 기다리면 포기합니다)

확인 모드는 `server/mcp_config.json`의 `approval`을 수정한 뒤 서버를 다시 시작하면 바뀝니다: `readonly`(기본값: Gemini 웹 확인 + 컴퓨터 확인 창), `all`(읽기 전용 작업도 창 표시), `none`(Gemini 웹 확인만 사용, 컴퓨터 창은 표시하지 않음).

### 보안

- 연결하려면 페어링 코드를 입력해야 하며, 5번 틀리면 10분 동안 잠깁니다.
- `server/mcp_config.json`(페어링 코드, 보안 비밀)과 `server/mcp_auth.json`(로그인 자격 증명)은 다른 사람과 공유하지 마세요.
- 연결된 모든 기기를 무효화하려면: 서버를 끄고 `server/mcp_auth.json`을 삭제한 뒤 다시 시작합니다. 페어링 코드를 바꾸려면: `mcp_config.json`에서 `pairing_code` 줄을 삭제하고 다시 시작합니다.
- 터널을 열면 누구나 이 URL에 접속할 수 있지만, 페어링 코드 없이는 어떤 도구도 사용할 수 없습니다.

## 패널 안내

Gemini 페이지 오른쪽 아래의 **⚒ GeminiForge**를 펼치고 "언어" 메뉴에서 **English, 繁體中文, 简体中文, 日本語, 한국어**를 전환합니다. 처음 열 때는 Chrome 인터페이스 언어를 따르며, 그 밖의 언어는 기본적으로 영어를 사용합니다. 선택한 언어는 현재 Chrome 프로필에 저장되고 열려 있는 Gemini 탭에도 동기화됩니다.

패널, 확인 버튼, 이후에 삽입되는 도구 안내는 모두 선택한 언어를 사용합니다. 로컬 서버가 연결되면 다음에 열리는 MCP 페어링 페이지와 데스크톱 확인 창에도 이 설정이 적용됩니다. 기존 설치를 업데이트할 때는 확장 프로그램을 다시 로드하고, Gemini 탭을 새로고침하고, `start.bat`의 로컬 서버를 다시 시작하세요. MCP도 사용한다면 `start_mcp.bat`도 함께 다시 시작하세요.

| 항목 | 기능 |
|---|---|
| 동그라미 | 초록색: 서버 연결됨, 빨간색: `start.bat`을 실행하세요 |
| 사용 | 끄면 확장 프로그램이 어떤 도구도 실행하지 않습니다 |
| 새 채팅에 도구 안내 추가 | 새 채팅에서 첫 메시지를 보낼 때 도구 안내를 자동으로 앞에 붙입니다 |
| 모드 | **단계마다 확인**: 가장 안전<br>**읽기 전용 도구 외에는 확인** (기본값): 파일 보기, 스크린샷은 자동 실행. 파일 쓰기, 명령 실행, 마우스 클릭은 "실행"을 눌러야 함<br>**모두 자동 실행**: 완전히 맡김. 신뢰하는 작업에서만 사용하세요 |
| 중지 | 현재 진행 중인 흐름을 즉시 중단합니다 |
| 확인 창의 "이 채팅에서 허용" | 같은 채팅에서는 이 도구를 다시 묻지 않습니다(채팅을 바꾸면 초기화) |
| 스크린샷 테스트 | 스크린샷과 업로드가 정상인지 확인합니다 |

자동 실행이 40회 연속되면 멈추고 계속할지 묻습니다.

## 부팅 시 자동 시작 (선택)

- `autostart_on.bat` 더블클릭: 이후 컴퓨터를 켜면 서버가 최소화된 창으로 자동 시작됩니다
- `autostart_off.bat` 더블클릭: 해제

## Gemini가 사용할 수 있는 도구

- **파일과 프로그램**: list_dir, read_file, write_file, edit_file, search, find_files, run(PowerShell), start_process, open, 클립보드
- **백업과 복원**: list_backups, restore_backup (아래 "파일 백업" 참고)
- **컴퓨터 조작**: screenshot, zoom(작은 글자 확대), click, double_click, right_click, move, drag, type, key, scroll
- **정확한 클릭**: list_windows, focus_window, ui_elements(Windows 접근성 인터페이스로 버튼과 링크의 이름과 위치를 나열), click_element(이름이나 번호로 클릭하며 버튼을 직접 실행: 마우스를 움직이지 않고, 창이 백그라운드에 있어도 되며, 창에 표시된 텍스트를 돌려줌)

`ui_elements`는 `window_id`를 반환합니다. 이후 작업에 같은 ID를 넘기면 이름이 같은 브라우저 탭을 잘못 고르는 일을 막을 수 있습니다. 이미 확인한 일련의 버튼은 `click_elements {window_id, names}`로 차례대로 실행할 수 있으며(최대 16개), 한 단계라도 실패하면 멈춥니다. MCP 모드에서는 이 일련의 작업 전체를 여전히 Gemini 웹 페이지에서 확인하며, 웹 쪽 확인이 없어지지는 않습니다. 창이나 요소가 더 이상 유효하지 않으면 도구가 오류를 알리고 다시 확인하도록 요청하며, 예전 좌표로 클릭하지 않습니다.

도구 안내는 `extension/prompts/`에 언어별로 한 파일씩 있으며, 직접 수정할 수 있습니다.

## 파일 백업

Gemini가 `write_file` 또는 `edit_file`로 파일을 바꾸기 전에 원본을 `GeminiForge/backups/`에 복사하고, 결과에 백업 번호를 함께 알려 줍니다.

- **Gemini에게 복원 요청**: "방금 바꾼 파일을 되돌려 줘"라고 말하면 `list_backups`로 번호를 찾고 `restore_backup`으로 복원합니다.
- **직접 복원**: `restore.bat`을 더블클릭하면 최근 30개의 백업이 나열됩니다. 번호를 입력하고 Enter를 누르면 됩니다.
- 원래 없던 파일, 즉 Gemini가 새로 만든 파일은 복원하면 삭제됩니다.
- 복원하기 전에 현재 내용도 백업하므로, 잘못 복원해도 다시 되돌릴 수 있습니다.
- 30일 동안, 최대 5000개까지 보관합니다. 50 MB를 넘는 파일은 기록만 하고 내용은 백업하지 않습니다.
- **`run`으로 실행한 명령이 변경한 파일은 백업되지 않습니다** (예: PowerShell로 파일 삭제, `git checkout`).

## 팁

- **내가 컴퓨터를 함께 쓰고 있을 때**: click, type, key는 실제 마우스와 키보드를 사용하므로 내 조작과 충돌할 수 있습니다. click_element는 대부분의 버튼을 직접 실행하므로 방해가 되지 않습니다.
- **다중 모니터**: 스크린샷은 기본적으로 마우스가 있는 모니터를 찍습니다. 요소를 클릭하거나 창을 전환한 뒤에는 그 창이 있는 모니터를 자동으로 찍습니다.

## 안전 수칙

- **마우스 조작 긴급 정지**: 마우스를 화면의 아무 모서리로 세게 밀면 진행 중인 작업이 즉시 중단됩니다.
- Gemini는 자신이 본 웹 페이지와 파일 내용을 읽게 되며, 그 안에 악의적인 지시가 숨어 있을 수 있습니다. "모두 자동 실행"을 사용할 때는 특히 주의하세요.
- 서버는 이 컴퓨터의 Chrome 확장 프로그램이 보낸 요청만 받으며, 일반 웹 페이지는 서버를 호출할 수 없습니다.
- 읽은 파일과 스크린샷 내용은 Google의 Gemini로 전송됩니다.

## 폴더 구조

```
GeminiForge/
├─ start.bat              확장 프로그램용 서버 시작 (방법 1)
├─ start_mcp.bat          MCP 서버 시작 (방법 2)
├─ autostart_on.bat       부팅 시 자동 시작 설정
├─ autostart_off.bat      부팅 시 자동 시작 해제
├─ restore.bat            백업 목록을 보여 주고 파일 복원
├─ backups/               자동 백업 (git에 포함하지 않음)
├─ server/
│  ├─ server.py           모든 도구의 구현이자 확장 프로그램용 서버
│  ├─ mcp_server.py       MCP 서버 (OAuth 인증 + 확인 창)
│  ├─ approve_dialog.py   컴퓨터의 확인 창
│  ├─ backups.py          변경 전 자동 백업과 복원
│  └─ requirements.txt
└─ extension/             Chrome 확장 프로그램
   ├─ manifest.json
   ├─ background.js       요청을 로컬 서버로 전달
   ├─ content.js          Gemini 페이지에서: 패널, 도구 명령 감지, 결과 반환
   └─ prompts/            Gemini용 도구 안내, 언어별 파일 하나씩 (수정 가능)
      └─ zh-TW.md, zh-CN.md, en.md, ja.md, ko.md
```

## 자주 묻는 질문

- **동그라미가 빨간색**: `start.bat`이 실행되지 않았거나 닫혔습니다.
- **포트 18765가 사용 중**: `start.bat`의 `set PYTHONIOENCODING` 다음 줄에 `set GF_PORT=다른 포트 번호`를 추가하고, `extension/manifest.json`과 `extension/background.js`의 18765도 함께 바꾼 뒤 `chrome://extensions`에서 확장 프로그램을 다시 로드하세요.
- **확장 프로그램 코드(content.js 등)를 수정함**: `chrome://extensions`에서 GeminiForge 카드의 다시 로드 ↻를 누른 뒤 Gemini를 새로고침하세요.
- **Gemini 업데이트 후 확장 프로그램이 작동하지 않음**: Gemini 웹 페이지 구조가 바뀐 것이므로 `content.js`에서 입력창과 전송 버튼을 찾는 부분을 업데이트해야 합니다.
- **@GeminiForge를 확인 없이 완전 자동으로 쓸 수 있나요?** 일부만 가능합니다:
  - **컴퓨터의 확인 창**: 끌 수 있습니다. `server/mcp_config.json`의 `approval`을 `none`으로 바꾸고 서버를 다시 시작하세요.
  - **Gemini 웹 페이지의 "Allow / Deny"**: 끌 수 없습니다. Google의 안전 장치로, 무언가를 변경하는 도구(프로그램 열기, 클릭, 파일 수정)는 매번 묻습니다.
  - 모든 도구를 "읽기 전용"으로 보고해서 웹 확인을 건너뛸 수는 있습니다: `server/mcp_config.json`에 `"web_confirm": "off"`를 설정하고 서버를 다시 시작하세요(기본값 `on`). **위험은 본인 책임입니다**: 이는 Google의 안전 설계를 우회하는 것과 같습니다. Gemini가 읽은 웹 페이지나 파일에 악의적인 지시(예: "어떤 폴더를 삭제해")가 숨어 있으면 확인 없이 바로 컴퓨터에서 실행됩니다. 직접 지켜보고 있을 때만 사용하는 것을 권장합니다.
  - 완전 자동에 가장 가까운 방법: @GeminiForge를 사용하고 `approval`을 `none`으로 설정한 뒤, 무언가를 변경하는 단계마다 Gemini 웹 페이지에서 "Allow"를 한 번씩 누르기. 또는 Chrome 확장 프로그램(Chat 모드)의 "모두 자동 실행"을 사용하면 전혀 누를 필요가 없지만, 직접 지켜보고 있을 때만 사용하는 것이 좋습니다.

## 라이선스

[Fluxenith](https://github.com/Fluxenith)가 유지 관리합니다. MIT 라이선스이며, 자세한 내용은 [LICENSE](LICENSE)를 참고하세요. 이 프로젝트는 Google과 제휴하거나 Google의 보증을 받은 것이 아닙니다.
