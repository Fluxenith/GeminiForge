#!/bin/sh
# Lists the file backups and restores one (macOS, Linux).
GF_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$GF_DIR/server/lang.sh"
. "$GF_DIR/server/exe.sh"
cd "$GF_DIR/server" || exit 1
gf_find_exe || exit 1
echo "$MSG_RESTORE_TITLE"
echo
"$EXE" backups list 30
echo
printf '%s' "$MSG_RESTORE_ASK"
read -r ID
if [ -n "$ID" ]; then
  "$EXE" backups restore "$ID"
fi
