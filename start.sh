#!/bin/sh
# Starts the GeminiForge local server for the Chrome extension (macOS, Linux).
GF_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$GF_DIR/server/lang.sh"
. "$GF_DIR/server/exe.sh"
printf '\033]0;%s\007' "$MSG_SERVER_TITLE"
cd "$GF_DIR/server" || exit 1
# Fast waits: go on as soon as the program has reacted instead of always
# waiting the fixed time. Change the 1 to 0 for fixed waits.
: "${GF_FAST_WAIT:=1}"
export GF_FAST_WAIT
gf_find_exe || exit 1
exec "$EXE"
