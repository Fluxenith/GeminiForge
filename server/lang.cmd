@echo off
rem Sets GF_LANG and the MSG_* texts used by the .bat scripts.
rem Language: the one picked in the extension panel, else the Windows display language.
rem Callers run "chcp 65001" first so this UTF-8 file is read correctly.
rem Keep ASCII parentheses out of the texts: they would break the callers' if blocks.
set "GF_LANG="
if exist "%LOCALAPPDATA%\GeminiForge\language.txt" set /p GF_LANG=<"%LOCALAPPDATA%\GeminiForge\language.txt"
if not defined GF_LANG for /f "delims=" %%L in ('powershell -NoProfile -Command "(Get-UICulture).Name"') do set "GF_LANG=%%L"
set "GF_CODE=%GF_LANG%"
set "GF_LANG=en"
if /i "%GF_CODE:~0,2%"=="ja" set "GF_LANG=ja"
if /i "%GF_CODE:~0,2%"=="ko" set "GF_LANG=ko"
if /i "%GF_CODE:~0,2%"=="zh" set "GF_LANG=zh-CN"
if /i "%GF_CODE:~0,2%"=="zh" echo %GF_CODE%| findstr /i "TW HK MO Hant" >nul && set "GF_LANG=zh-TW"
set "GF_CODE="
goto :%GF_LANG%

:zh-TW
set "MSG_SERVER_TITLE=GeminiForge 本機伺服器"
set "MSG_MCP_TITLE=GeminiForge MCP 伺服器"
set "MSG_FIRST_RUN=第一次執行：正在安裝需要的套件，請稍候..."
set "MSG_NO_PYTHON=找不到 Python，請先安裝 Python 3.10 以上版本。"
set "MSG_PIP_FAILED=套件安裝失敗，請檢查網路後再試一次。"
set "MSG_AUTO_ON=已設定：之後開機會自動在背景（最小化）啟動 GeminiForge 伺服器。"
set "MSG_AUTO_ON_HINT=要取消請執行 autostart_off.bat。"
set "MSG_AUTO_FAIL=設定失敗。"
set "MSG_AUTO_OFF=已取消開機自動啟動。"
set "MSG_RESTORE_TITLE=GeminiForge 檔案備份：Gemini 用 write_file / edit_file 改過的檔案"
set "MSG_RESTORE_ASK=輸入要復原的備份編號，直接按 Enter 取消："
goto :eof

:zh-CN
set "MSG_SERVER_TITLE=GeminiForge 本地服务器"
set "MSG_MCP_TITLE=GeminiForge MCP 服务器"
set "MSG_FIRST_RUN=首次运行：正在安装所需的依赖包，请稍候..."
set "MSG_NO_PYTHON=找不到 Python，请先安装 Python 3.10 或更高版本。"
set "MSG_PIP_FAILED=依赖包安装失败，请检查网络后重试。"
set "MSG_AUTO_ON=已设置：以后开机会自动在后台（最小化）启动 GeminiForge 服务器。"
set "MSG_AUTO_ON_HINT=要取消请运行 autostart_off.bat。"
set "MSG_AUTO_FAIL=设置失败。"
set "MSG_AUTO_OFF=已取消开机自动启动。"
set "MSG_RESTORE_TITLE=GeminiForge 文件备份：Gemini 用 write_file / edit_file 改过的文件"
set "MSG_RESTORE_ASK=输入要恢复的备份编号，直接按 Enter 取消："
goto :eof

:ja
set "MSG_SERVER_TITLE=GeminiForge ローカルサーバー"
set "MSG_MCP_TITLE=GeminiForge MCP サーバー"
set "MSG_FIRST_RUN=初回実行：必要なパッケージをインストールしています。しばらくお待ちください..."
set "MSG_NO_PYTHON=Python が見つかりません。Python 3.10 以降をインストールしてください。"
set "MSG_PIP_FAILED=パッケージのインストールに失敗しました。ネットワークを確認してもう一度お試しください。"
set "MSG_AUTO_ON=設定しました：次回から Windows 起動時に GeminiForge サーバーがバックグラウンド（最小化）で起動します。"
set "MSG_AUTO_ON_HINT=取り消すには autostart_off.bat を実行してください。"
set "MSG_AUTO_FAIL=設定に失敗しました。"
set "MSG_AUTO_OFF=自動起動を解除しました。"
set "MSG_RESTORE_TITLE=GeminiForge ファイルのバックアップ：Gemini が write_file / edit_file で変更したファイル"
set "MSG_RESTORE_ASK=復元するバックアップ ID を入力、Enter でキャンセル："
goto :eof

:ko
set "MSG_SERVER_TITLE=GeminiForge 로컬 서버"
set "MSG_MCP_TITLE=GeminiForge MCP 서버"
set "MSG_FIRST_RUN=첫 실행: 필요한 패키지를 설치하는 중입니다. 잠시 기다려 주세요..."
set "MSG_NO_PYTHON=Python을 찾을 수 없습니다. Python 3.10 이상을 먼저 설치하세요."
set "MSG_PIP_FAILED=패키지 설치에 실패했습니다. 네트워크를 확인한 뒤 다시 시도하세요."
set "MSG_AUTO_ON=설정 완료: 이제 Windows를 켤 때 GeminiForge 서버가 백그라운드에서 최소화된 상태로 시작됩니다."
set "MSG_AUTO_ON_HINT=취소하려면 autostart_off.bat을 실행하세요."
set "MSG_AUTO_FAIL=설정에 실패했습니다."
set "MSG_AUTO_OFF=자동 시작을 해제했습니다."
set "MSG_RESTORE_TITLE=GeminiForge 파일 백업: Gemini가 write_file / edit_file로 바꾼 파일"
set "MSG_RESTORE_ASK=복원할 백업 ID 입력, Enter를 누르면 취소: "
goto :eof

:en
set "MSG_SERVER_TITLE=GeminiForge local server"
set "MSG_MCP_TITLE=GeminiForge MCP server"
set "MSG_FIRST_RUN=First run: installing the required packages, please wait..."
set "MSG_NO_PYTHON=Python not found. Install Python 3.10 or later first."
set "MSG_PIP_FAILED=Package installation failed. Check your network and try again."
set "MSG_AUTO_ON=Done: the GeminiForge server will now start minimized in the background when Windows starts."
set "MSG_AUTO_ON_HINT=To turn this off, run autostart_off.bat."
set "MSG_AUTO_FAIL=Setup failed."
set "MSG_AUTO_OFF=Autostart turned off."
set "MSG_RESTORE_TITLE=GeminiForge file backups: files Gemini changed with write_file / edit_file"
set "MSG_RESTORE_ASK=Backup id to restore, or press Enter to cancel: "
goto :eof
