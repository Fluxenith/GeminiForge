#!/bin/sh
# Undoes autostart_on.sh: the server no longer starts at login.
GF_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$GF_DIR/server/lang.sh"
if [ "$(uname)" = Darwin ]; then
  rm -f "$HOME/Library/LaunchAgents/com.geminiforge.server.plist"
else
  rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/geminiforge.desktop"
fi
echo "$MSG_AUTO_OFF"
