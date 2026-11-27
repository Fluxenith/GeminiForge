# Defines gf_find_exe: sets EXE to the GeminiForge server program, building
# it the first time. Callers set GF_DIR to the GeminiForge folder and source
# lang.sh first. gf_find_exe returns 1 when there is no program.

gf_find_exe() {
  EXE="$GF_DIR/server/target/release/geminiforge-server"
  [ -x "$EXE" ] && return 0
  EXE="$GF_DIR/server/geminiforge-server"
  if [ -f "$EXE" ]; then
    # unzip tools do not always keep the executable bit
    [ -x "$EXE" ] || chmod +x "$EXE"
    return 0
  fi
  EXE="$GF_DIR/server/target/release/geminiforge-server"
  [ -x "$HOME/.cargo/bin/cargo" ] && PATH="$HOME/.cargo/bin:$PATH"
  if ! command -v cargo >/dev/null 2>&1; then
    echo "$MSG_NO_CARGO"
    return 1
  fi
  echo "$MSG_FIRST_RUN"
  if ! (cd "$GF_DIR/server" && cargo build --release); then
    echo "$MSG_BUILD_FAILED"
    return 1
  fi
  return 0
}
