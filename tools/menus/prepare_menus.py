"""Convert the retail front-end (data/big/fedata.big APT screens) for the menu runtime.

  python tools/menus/prepare_menus.py --game ~/skate3-disc --output <install>/assets/private/menus

Writes one JSON per screen, keyed by its movie path as the scripts name it
("source/screens/main/core_menu.swf" -> source/screens/main/core_menu.json):
timeline characters from the APT, every script block the timeline runs, and
the imports/exports that link screens to the shared controls in main.apt.
Shapes and bitmaps come in a later stage (the HUD's SceneFlattener).
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

for root in Path(__file__).resolve().parents[1:3]:
    sys.path.insert(0, str(root))
from owned_game.big import BigArchive  # noqa: E402
from vendor.skate3_ui.apt import inspect_apt  # noqa: E402
from menus.apt_actions import MenuActions  # noqa: E402

PREFIX = 'data/fe/'


def script_offsets(node, found):
    """Offsets of timeline scripts (frame actions and init actions)."""
    if isinstance(node, dict):
        if node.get('type_name') in ('do_action', 'do_init_action') and node.get('actions_offset'):
            found.add(node['actions_offset'])
        for value in node.values():
            script_offsets(value, found)
    elif isinstance(node, list):
        for value in node:
            script_offsets(value, found)
    return found


# Flash ClipEventFlags as one big-endian word (onClipEvent(load) = 0x01000000).
CLIP_EVENTS = {0x01000000: 'load', 0x02000000: 'enterFrame', 0x04000000: 'unload', 0x08000000: 'mouseMove',
               0x10000000: 'mouseDown', 0x20000000: 'mouseUp', 0x40000000: 'keyDown', 0x80000000: 'keyUp',
               0x00010000: 'data', 0x00020000: 'initialize', 0x00040000: 'press', 0x00080000: 'release',
               0x00100000: 'releaseOutside', 0x00200000: 'rollOver', 0x00400000: 'rollOut',
               0x00800000: 'dragOver', 0x00000100: 'dragOut', 0x00000200: 'keyPress', 0x00000400: 'construct'}


def clip_actions(node, raw, found):
    """Placements with onClipEvent/on() handlers (flag 0x80): actions_offset
    points at (count, pointer) and 12-byte records (event flags, key code,
    script offset). Records are attached to the placement as clip_actions."""
    if isinstance(node, dict):
        if node.get('flags', 0) & 0x80 and node.get('actions_offset'):
            at = node['actions_offset']
            count = int.from_bytes(raw[at:at + 4], 'big')
            table = int.from_bytes(raw[at + 4:at + 8], 'big')
            records = []
            for i in range(min(count, 64)):
                r = table + i * 12
                flags, key, script = (int.from_bytes(raw[r + k:r + k + 4], 'big') for k in (0, 4, 8))
                records.append({'flags': flags, 'key': key, 'actions_offset': script,
                                'events': [n for bit, n in CLIP_EVENTS.items() if flags & bit]})
                found.add(script)
            node['clip_actions'] = records
            node['actions_offset'] = 0
        for value in node.values():
            clip_actions(value, raw, found)
    elif isinstance(node, list):
        for value in node:
            clip_actions(value, raw, found)
    return found


def convert(apt_path: Path, const_path: Path, movie: str):
    data = inspect_apt(apt_path, const_path)
    actions = MenuActions(apt_path.read_bytes(), const_path.read_bytes())
    handlers = clip_actions(data, apt_path.read_bytes(), set())
    blocks = {str(offset): actions.stream(offset) for offset in sorted(script_offsets(data, set()) | handlers)}
    return {
        'format': 'skate3-menu-screen', 'version': 1, 'movie': movie,
        'source': {'apt_sha256': hashlib.sha256(apt_path.read_bytes()).hexdigest(),
                   'const_sha256': hashlib.sha256(const_path.read_bytes()).hexdigest()},
        'root': {k: v for k, v in data['root'].items() if k != 'movie'},
        'characters': data['characters'],
        'imports': data['imports'], 'exports': data['exports'],
        'actions': blocks,
    }


def prepare(game: Path, output: Path, work: Path, report=print):
    archive = BigArchive(game / 'data/big/fedata.big')
    screens = {}
    for entry in archive.entries:
        name = entry.path.replace('\\', '/')
        stem, _, ext = name.rpartition('.')
        if ext in ('apt', 'const'):
            raw = work / name
            raw.parent.mkdir(parents=True, exist_ok=True)
            raw.write_bytes(archive.read(entry))
            screens.setdefault(stem, {})[ext] = raw
    written, failed = 0, {}
    for stem, parts in sorted(screens.items()):
        if 'apt' not in parts or 'const' not in parts:
            continue
        movie = stem.removeprefix(PREFIX) + '.swf'
        try:
            screen = convert(parts['apt'], parts['const'], movie)
        except Exception as error:  # keep converting; the runtime reports what's missing
            failed[movie] = str(error)
            continue
        target = output / (stem.removeprefix(PREFIX) + '.json')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(screen, separators=(',', ':')), encoding='utf-8')
        written += 1
    (output / 'index.json').write_text(json.dumps({'version': 1, 'screens': written, 'failed': failed}, indent=1))
    report(f'Menus: {written} screens converted, {len(failed)} failed')
    for movie, error in failed.items():
        report(f'  {movie}: {error}')
    return written, failed


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--game', type=Path, required=True, help='Extracted Skate 3 disc')
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--work', type=Path, default=Path.home() / '.cache/skate3rust-menus')
    args = p.parse_args()
    prepare(args.game.expanduser(), args.output.expanduser(), args.work.expanduser())
