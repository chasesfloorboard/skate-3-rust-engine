#!/usr/bin/env bash
# Linux counterpart of Build-Release.ps1: produces target/skate3rust-linux-x64.tar.gz.
# Everything is built inside the Steam Runtime 3 (sniper) SDK so the game and
# setup helper run on any distro with glibc >= 2.31. Needs Docker and network.
# No game files are included; players select their own ISO on first launch.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="${SKATE_BUILD_IMAGE:-registry.gitlab.steamos.cloud/steamrt/sniper/sdk:latest}"

if [[ "${1:-}" != --inside ]]; then
    mkdir -p "$root/target-steamrt/home"
    exec docker run --rm --user "$(id -u):$(id -g)" -v "$root:/src" -w /src \
        -e HOME=/src/target-steamrt/home -e GITHUB_RUN_NUMBER -e RELEASE_TAG \
        "$image" bash scripts/build-release-linux.sh --inside
fi

cd /src
export CARGO_HOME="$HOME/.cargo" RUSTUP_HOME="$HOME/.rustup" UV_CACHE_DIR="$HOME/.cache/uv"
export PATH="$CARGO_HOME/bin:$HOME/.local/bin:$PATH"
target=/src/target-steamrt
stage_parent="$target/release-package"
stage="$stage_parent/skate3rust-linux-x64"
downloads="$target/downloads"
mkdir -p "$downloads" "$target/native"

fetch() { # url sha256 destination
    if [[ ! -f "$3" ]] || ! echo "$2  $3" | sha256sum -c --quiet - 2>/dev/null; then
        curl -sSfL -o "$3.part" "$1"
        echo "$2  $3.part" | sha256sum -c --quiet -
        mv "$3.part" "$3"
    fi
}

# Toolchains live under target-steamrt/home so reruns are incremental.
if [[ ! -x "$CARGO_HOME/bin/cargo" ]]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
fi
if [[ ! -x "$HOME/.local/bin/uv" ]]; then
    curl -LsSf https://astral.sh/uv/install.sh | env UV_NO_MODIFY_PATH=1 sh
fi

echo "== Game"
cargo build --release --locked --target-dir "$target" -p skate-game --bin skate3rust --no-default-features
rustc --edition 2024 --crate-type cdylib -C opt-level=3 -C panic=abort \
    tools/asset_pipeline/refpack_native.rs -o "$target/native/refpack.so"

echo "== extract-xiso"
# Same XboxDev build the Windows setup downloads, compiled from source.
xiso_tag=build-202505152050
xiso="$target/extract-xiso-$xiso_tag"
if [[ ! -x "$xiso/build/extract-xiso" ]]; then
    rm -rf "$xiso"
    git clone --quiet --depth 1 --branch "$xiso_tag" https://github.com/XboxDev/extract-xiso.git "$xiso"
    cmake -S "$xiso" -B "$xiso/build" -DCMAKE_BUILD_TYPE=Release >/dev/null
    cmake --build "$xiso/build" -j"$(nproc)" >/dev/null
fi

echo "== vgmstream"
vgm_tag=r2117
fetch "https://github.com/vgmstream/vgmstream/releases/download/$vgm_tag/vgmstream-linux.zip" \
    2f98c77f756079f63fbd119939067f1ed461d77e70993bc4cc372736d859c84a "$downloads/vgmstream-linux-$vgm_tag.zip"
curl -sSfL -o "$downloads/vgmstream-COPYING" "https://raw.githubusercontent.com/vgmstream/vgmstream/$vgm_tag/COPYING"

echo "== Setup helper"
venv="$target/package-venv"
[[ -x "$venv/bin/python" ]] || uv venv --quiet --python 3.12 --python-preference only-managed "$venv"
uv pip install --quiet --python "$venv/bin/python" -r tools/requirements-setup.txt
python="$venv/bin/python"
"$python" -m unittest tools.test_setup_assets tools.asset_pipeline.test_marquee_assets \
    tools.asset_pipeline.test_customiser_setup tools.asset_pipeline.test_setup_recovery \
    tools.asset_pipeline.test_versions tools.asset_pipeline.test_optional_content

rm -rf "$stage_parent"
mkdir -p "$stage/support" "$stage/mods" "$stage/logs" "$stage/docs/images" "$stage/licenses"
source_stage="$stage_parent/setup-source"
# Same source selection as Build-Release.ps1.
"$python" - "$source_stage/tools" <<'EOF'
import re,shutil,sys
from pathlib import Path
tools=Path('tools');destination=Path(sys.argv[1])
skip={'asset_pipeline/build_map.py','asset_pipeline/finish_character.py',
      'add_onboard_ik_targets.py','apply_default_skater_materials.py','export_bevy_glb.py'}
for source in tools.rglob('*'):
    if not source.is_file() or '__pycache__' in source.parts:continue
    if source.suffix not in {'.py','.json','.txt','.md','.toml','.rs'} and source.name!='LICENSE':continue
    name=source.relative_to(tools).as_posix()
    if re.fullmatch(r'mixamo_to_skate/[^/]*\.json',name) or re.search(r'(^|/)blender[^/]*(/|$)',name) or name in skip:continue
    (destination/name).parent.mkdir(parents=True,exist_ok=True)
    shutil.copy2(source,destination/name)
EOF
# The Windows build also bundles the FBX2glTF character importer; the Linux
# package does not yet, so Custom Models import is unavailable there.
"$python" -m PyInstaller --noconfirm --clean --onefile --log-level WARN --name skate3setup \
    --paths /src --hidden-import numpy --hidden-import PIL.Image --hidden-import tkinter \
    --add-binary "$target/native/refpack.so:tools/asset_pipeline" \
    --exclude-module bpy --exclude-module mathutils \
    --copy-metadata numpy --copy-metadata Pillow --copy-metadata PyInstaller \
    --add-data "$source_stage/tools:tools" --add-data "/src/docs/images/skating-crab.png:docs/images" \
    --distpath "$stage/support" --workpath "$target/setup-build/work" --specpath "$target/setup-build" tools/setup.py
install -m 755 "$xiso/build/extract-xiso" "$stage/support/extract-xiso"
unzip -q -o -d "$stage/support" "$downloads/vgmstream-linux-$vgm_tag.zip" vgmstream-cli
chmod 755 "$stage/support/vgmstream-cli"

echo "== Package"
strip -o "$stage/skate3rust" "$target/release/skate3rust"
install -m 755 scripts/linux-package/Play.sh scripts/linux-package/Install-Shortcut.sh "$stage/"
cp mods/native-trainer.zip mods/mario-kart.zip mods/README.md "$stage/mods/"
cp README.md docs/THIRD_PARTY_NOTICES.md scripts/linux-package/README-LINUX.md "$stage/"
cp docs/images/skating-crab.png "$stage/docs/images/"
cp docs/installation.md docs/retail-renderer.md docs/crash-reports.md docs/performance-tracing.md \
    docs/custom-models.md docs/character-customisation.md "$stage/docs/"
cp tools/vendor/utt/LICENSE "$stage/licenses/UTT.txt"
cp tools/vendor/university/LICENSE-PROJECT.md "$stage/licenses/CustomEngineLayer.txt"
cp vendor/bevy_pbr/LICENSE-MIT "$stage/licenses/Bevy-MIT.txt"
cp vendor/bevy_pbr/LICENSE-APACHE "$stage/licenses/Bevy-APACHE.txt"
cp "$xiso/LICENSE.TXT" "$stage/licenses/extract-xiso.txt" 2>/dev/null || cp "$xiso"/LICENSE* "$stage/licenses/extract-xiso.txt"
cp "$downloads/vgmstream-COPYING" "$stage/licenses/vgmstream.txt"
"$python" -c 'import sys,pathlib; print(pathlib.Path(sys.base_prefix,"lib/python3.12/LICENSE.txt").read_text())' > "$stage/licenses/Python.txt"

"$python" - "$stage" "$source_stage/tools" <<'EOF'
import hashlib,json,os,subprocess,sys
from pathlib import Path
stage=Path(sys.argv[1]);tools=sys.argv[2]
run=lambda *a:subprocess.run(a,check=True,capture_output=True,text=True).stdout.strip()
files={p.relative_to(stage).as_posix():hashlib.sha256(p.read_bytes()).hexdigest()
       for p in sorted(stage.rglob('*')) if p.is_file() and not p.relative_to(stage).as_posix().startswith('mods/')}
release={'schema':1,'repository':'SK8-ENGINE/skate-3-rust-engine','target':'linux-x64',
    'build':int(os.environ.get('GITHUB_RUN_NUMBER') or 0),'tag':os.environ.get('RELEASE_TAG') or 'development',
    'revision':run('git','rev-parse','HEAD'),'files':files,
    'asset_pipelines':json.loads(run(sys.executable,f'{tools}/asset_pipeline/versions.py','--tools',tools)),
    'character_customiser':run(sys.executable,'-m','tools.asset_pipeline.customiser_setup','--fingerprint')}
(stage/'release.json').write_text(json.dumps(release,indent=2),encoding='utf-8')
EOF

archive=/src/target/skate3rust-linux-x64.tar.gz
mkdir -p /src/target
tar -C "$stage_parent" -czf "$archive" skate3rust-linux-x64
(cd /src/target && sha256sum skate3rust-linux-x64.tar.gz > skate3rust-linux-x64.tar.gz.sha256)
echo "Release package: target/skate3rust-linux-x64.tar.gz"
