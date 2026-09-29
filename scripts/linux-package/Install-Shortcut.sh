#!/usr/bin/env bash
# Adds "Skate 3 Rust Engine" to your application menu, pointing at this folder.
# Run it again after moving the folder. ./Install-Shortcut.sh --remove undoes it.
set -euo pipefail
here="$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
applications="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
entry="$applications/skate3rust.desktop"
if [[ "${1:-}" == --remove ]]; then
    rm -f "$entry"; echo "Removed $entry"; exit 0
fi
mkdir -p "$applications"
cat > "$entry" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Skate 3 Rust Engine
Comment=Skate 3 on a native engine, using your own game disc
Exec="$here/Play.sh"
Path=$here
Icon=$here/docs/images/skating-crab.png
Terminal=false
Categories=Game;SportsGame;
DESKTOP
update-desktop-database "$applications" 2>/dev/null || true
echo "Added Skate 3 Rust Engine to your application menu ($entry)."
