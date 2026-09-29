#!/usr/bin/env bash
# Skate 3 Rust Engine for Linux. The first launch opens setup, which asks for
# your own Skate 3 Xbox 360 ISO (or default.xex in an extracted copy) and
# converts it into this folder's data directory. Plug a controller in any time.
# Extra arguments pass straight through, e.g. ./Play.sh --map data/installations/*/maps/MegaPark.skate
cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")"
# Launched from Steam: Steam Input hides the real pad behind a virtual one and
# only feeds it while Steam sees the game focused, which it can't detect for
# native Wayland windows on a desktop session. Run under XWayland instead.
if [[ -n "${DISPLAY:-}" && ( -n "${SteamGameId:-}${SteamAppId:-}${SteamEnv:-}" || "${LD_PRELOAD:-}" == *gameoverlayrenderer* ) ]]; then
    unset WAYLAND_DISPLAY
fi
mkdir -p logs
log="logs/game-$(date +%Y%m%d-%H%M%S).log"
echo "Starting Skate 3 Rust Engine. Esc opens difficulty/graphics/pause settings. Log: $log"
./skate3rust "$@" > >(tee "$log") 2>&1
status=$?
# Without a terminal (menu shortcut, file manager) errors would go unseen.
if (( status != 0 )) && [[ ! -t 1 ]]; then
    message="Skate 3 Rust Engine stopped (exit code $status).
$(grep -v REPORT_META "$log" | sed 's/\x1b\[[0-9;]*m//g' | tail -n 6)

Full log: $PWD/$log"
    if command -v zenity >/dev/null; then zenity --error --title "Skate 3 Rust Engine" --no-markup --text "$message"
    elif command -v kdialog >/dev/null; then kdialog --title "Skate 3 Rust Engine" --error "$message"
    fi
fi
exit $status
