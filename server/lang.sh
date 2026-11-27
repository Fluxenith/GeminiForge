# Sets GF_LANG and the MSG_* texts used by the .sh scripts (macOS, Linux).
# Language: the one picked in the extension panel, else the system language.
# The other scripts source this file.

GF_LANG=""
gf_pref="${XDG_CONFIG_HOME:-$HOME/.config}/GeminiForge/language.txt"
if [ -f "$gf_pref" ]; then
  GF_LANG=$(head -n 1 "$gf_pref" | tr -d '\r\n ')
fi
if [ -z "$GF_LANG" ] && [ "$(uname)" = Darwin ]; then
  # first of the preferred languages, e.g. zh-Hant-TW
  GF_LANG=$(defaults read -g AppleLanguages 2>/dev/null | sed -n '2p' | tr -d ' ",')
fi
if [ -z "$GF_LANG" ]; then
  GF_LANG="${LC_ALL:-${LC_MESSAGES:-$LANG}}"
fi
case "$GF_LANG" in
  ja*) GF_LANG=ja ;;
  ko*) GF_LANG=ko ;;
  zh*[Tt][Ww]* | zh*[Hh][Kk]* | zh*[Mm][Oo]* | zh*[Hh]ant*) GF_LANG=zh-TW ;;
  zh*) GF_LANG=zh-CN ;;
  *) GF_LANG=en ;;
esac

case "$GF_LANG" in
zh-TW)
  MSG_SERVER_TITLE="GeminiForge 本機伺服器"
  MSG_MCP_TITLE="GeminiForge MCP 伺服器"
  MSG_FIRST_RUN="第一次執行：正在建置伺服器，大約需要一分鐘..."
  MSG_NO_CARGO="找不到 server/geminiforge-server。請到 GitHub 的 Releases 下載這個系統的完整壓縮檔，或先到 https://rustup.rs 安裝 Rust 再自動建置。"
  MSG_BUILD_FAILED="建置失敗，請查看上面的錯誤訊息。"
  MSG_AUTO_ON="已設定：之後登入時會自動在背景啟動 GeminiForge 伺服器。"
  MSG_AUTO_ON_HINT="要取消請執行 autostart_off.sh。"
  MSG_AUTO_FAIL="設定失敗。"
  MSG_AUTO_OFF="已取消開機自動啟動。"
  MSG_RESTORE_TITLE="GeminiForge 檔案備份：Gemini 用 write_file / edit_file 改過的檔案"
  MSG_RESTORE_ASK="輸入要復原的備份編號，直接按 Enter 取消："
  ;;
zh-CN)
  MSG_SERVER_TITLE="GeminiForge 本地服务器"
  MSG_MCP_TITLE="GeminiForge MCP 服务器"
  MSG_FIRST_RUN="首次运行：正在构建服务器，大约需要一分钟..."
  MSG_NO_CARGO="找不到 server/geminiforge-server。请到 GitHub 的 Releases 下载这个系统的完整压缩包，或先到 https://rustup.rs 安装 Rust 再自动构建。"
  MSG_BUILD_FAILED="构建失败，请查看上面的错误信息。"
  MSG_AUTO_ON="已设置：以后登录时会自动在后台启动 GeminiForge 服务器。"
  MSG_AUTO_ON_HINT="要取消请运行 autostart_off.sh。"
  MSG_AUTO_FAIL="设置失败。"
  MSG_AUTO_OFF="已取消开机自动启动。"
  MSG_RESTORE_TITLE="GeminiForge 文件备份：Gemini 用 write_file / edit_file 改过的文件"
  MSG_RESTORE_ASK="输入要恢复的备份编号，直接按 Enter 取消："
  ;;
ja)
  MSG_SERVER_TITLE="GeminiForge ローカルサーバー"
  MSG_MCP_TITLE="GeminiForge MCP サーバー"
  MSG_FIRST_RUN="初回実行：サーバーをビルドしています。1 分ほどかかります..."
  MSG_NO_CARGO="server/geminiforge-server が見つかりません。GitHub の Releases からこのシステム用の zip をダウンロードするか、https://rustup.rs から Rust をインストールして自動ビルドしてください。"
  MSG_BUILD_FAILED="ビルドに失敗しました。上のエラーを確認してください。"
  MSG_AUTO_ON="設定しました：次回からログイン時に GeminiForge サーバーがバックグラウンドで起動します。"
  MSG_AUTO_ON_HINT="取り消すには autostart_off.sh を実行してください。"
  MSG_AUTO_FAIL="設定に失敗しました。"
  MSG_AUTO_OFF="自動起動を解除しました。"
  MSG_RESTORE_TITLE="GeminiForge ファイルのバックアップ：Gemini が write_file / edit_file で変更したファイル"
  MSG_RESTORE_ASK="復元するバックアップ ID を入力、Enter でキャンセル："
  ;;
ko)
  MSG_SERVER_TITLE="GeminiForge 로컬 서버"
  MSG_MCP_TITLE="GeminiForge MCP 서버"
  MSG_FIRST_RUN="처음 실행: 서버를 빌드하는 중입니다. 1분 정도 걸립니다..."
  MSG_NO_CARGO="server/geminiforge-server를 찾을 수 없습니다. GitHub Releases에서 이 시스템용 zip을 내려받거나, https://rustup.rs 에서 Rust를 설치해 자동으로 빌드하세요."
  MSG_BUILD_FAILED="빌드에 실패했습니다. 위의 오류 메시지를 확인하세요."
  MSG_AUTO_ON="설정 완료: 이제 로그인할 때 GeminiForge 서버가 백그라운드에서 시작됩니다."
  MSG_AUTO_ON_HINT="취소하려면 autostart_off.sh를 실행하세요."
  MSG_AUTO_FAIL="설정에 실패했습니다."
  MSG_AUTO_OFF="자동 시작을 해제했습니다."
  MSG_RESTORE_TITLE="GeminiForge 파일 백업: Gemini가 write_file / edit_file로 바꾼 파일"
  MSG_RESTORE_ASK="복원할 백업 ID 입력, Enter를 누르면 취소: "
  ;;
*)
  MSG_SERVER_TITLE="GeminiForge local server"
  MSG_MCP_TITLE="GeminiForge MCP server"
  MSG_FIRST_RUN="First run: building the server, this takes about a minute..."
  MSG_NO_CARGO="server/geminiforge-server not found. Download the zip for this system from the GitHub Releases page, or install Rust from https://rustup.rs to build it."
  MSG_BUILD_FAILED="Build failed. See the errors above."
  MSG_AUTO_ON="Done: the GeminiForge server will now start in the background when you log in."
  MSG_AUTO_ON_HINT="To turn this off, run autostart_off.sh."
  MSG_AUTO_FAIL="Setup failed."
  MSG_AUTO_OFF="Autostart turned off."
  MSG_RESTORE_TITLE="GeminiForge file backups: files Gemini changed with write_file / edit_file"
  MSG_RESTORE_ASK="Backup id to restore, or press Enter to cancel: "
  ;;
esac
