#!/bin/sh
# Starts the GeminiForge local server in the background from the next login
# on: a LaunchAgent on macOS, an autostart entry of the desktop session on Linux.
GF_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$GF_DIR/server/lang.sh"
. "$GF_DIR/server/exe.sh"
# find or build the server now, so the login does not have to
gf_find_exe || exit 1

if [ "$(uname)" = Darwin ]; then
  xml() { printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }
  agents="$HOME/Library/LaunchAgents"
  log="$HOME/Library/Logs/GeminiForge.log"
  mkdir -p "$agents" "$HOME/Library/Logs" || { echo "$MSG_AUTO_FAIL"; exit 1; }
  # PATH is kept so commands find the same programs as in this terminal
  cat > "$agents/com.geminiforge.server.plist" <<EOF || { echo "$MSG_AUTO_FAIL"; exit 1; }
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.geminiforge.server</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string>
    <string>$(xml "$GF_DIR/start.sh")</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>$(xml "$PATH")</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>$(xml "$log")</string>
  <key>StandardErrorPath</key>
  <string>$(xml "$log")</string>
</dict>
</plist>
EOF
else
  dir="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
  mkdir -p "$dir" || { echo "$MSG_AUTO_FAIL"; exit 1; }
  # quoted the way desktop entries want: \ " ` $ escaped, % doubled
  q=$(printf '%s' "$GF_DIR/start.sh" | sed -e 's/\\/\\\\\\\\/g' -e 's/["`$]/\\\\&/g' -e 's/%/%%/g')
  cat > "$dir/geminiforge.desktop" <<EOF || { echo "$MSG_AUTO_FAIL"; exit 1; }
[Desktop Entry]
Type=Application
Name=GeminiForge
Comment=GeminiForge local server
Exec=sh "$q"
Terminal=false
X-GNOME-Autostart-enabled=true
EOF
fi
echo "$MSG_AUTO_ON"
echo "$MSG_AUTO_ON_HINT"
