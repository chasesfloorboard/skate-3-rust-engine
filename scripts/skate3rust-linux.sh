#!/usr/bin/env bash
# Linux launcher for dev builds: the default `dev-dynamic` feature links Bevy as
# a shared library, so expose target/debug/deps and the Rust sysroot libs
# (the Windows Build.ps1 copies the equivalent DLLs next to the exe instead).
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
debug="$root/target/debug"
export LD_LIBRARY_PATH="$debug/deps:$debug:$(rustc --print sysroot)/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
# Without --assets, use the active installation published by prepare_assets.py
# (assets/private/installation.json -> installations/<id>/assets).
if [[ " $* " != *" --assets "* ]]; then
    marker="$root/assets/private/installation.json"
    [[ -f "$marker" ]] || { echo "No $marker; run tools/prepare_assets.py first" >&2; exit 1; }
    dir="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["directory"])' "$marker")"
    set -- --assets "$root/assets/private/$dir/assets" "$@"
fi
exec "$debug/skate3rust" "$@"
