"""Skate 2 pros and specials as native characters (Call Skater "Skate 2" sections).

Skate 2's data/content/marquee.big has Skate 3's Marquee layout (recipe XML,
per-slot rx2 models, rx2 textures) inside a classic BIGF archive whose files
are RefPack-compressed. This adapts the archive and runs the Skate 3 roster
builder (asset_pipeline/native_roster.py, left untouched because it is part of
the customiser fingerprint) on it. Entries get "s2_" keys, so they never share
an identity with the Skate 3 versions of the same pro, and "game": "Skate 2".

  python -m tools.skate2.roster --skate2 ~/Downloads/"Skate 2" --installation <install> [--only big_black]
"""
import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.asset_pipeline import native_roster  # noqa: E402
from tools.owned_game.refpack import decompress as refpack  # noqa: E402
from tools.skate2 import bigf  # noqa: E402

# Recipes that are not skaters (props, crowd, tutorial dummies, stand-ins).
SKIP_PREFIXES = ('ai_skater', 'z_', 'tutorial', 'crowd', 'cameraman', 'filmer')
SKIP_SUFFIXES = ('_prop', '_can')
LABELS = {'big_black': 'Big Black', 'dem_bones': 'Dem Bones', 'cuz': 'Cuz Parry',
          'attiba_jefferson': 'Atiba Jefferson', 'giovanni_reda': 'Giovanni Reda',
          'pj_ladd': 'PJ Ladd', 'rob_dyrdek': 'Rob Dyrdek', 'freeway': 'Freeway',
          'security_03': 'Security Guard', 'darren_navarette': 'Darren Navarrette'}
# Not pros: Skate 2's story cast and guest characters.
SPECIAL = {'big_black', 'dem_bones', 'cuz', 'freeway', 'giovanni_reda', 'attiba_jefferson', 'michael_burnette',
           'mike', 'sammy', 'seb', 'shingo', 'slappy', 'security_03'}


@dataclass
class Entry:
    path: str
    offset: int
    size: int


class Archive:
    """BigArchive look-alike over a Skate 2 BIGF file (RefPack inflated on read)."""

    def __init__(self, path):
        self.path = Path(path)
        self.entries = [Entry(name, offset, size) for name, offset, size in bigf.entries(self.path)]

    def read(self, entry):
        with self.path.open('rb') as f:
            f.seek(entry.offset)
            data = f.read(entry.size)
        if data[:2] == b'\x10\xfb':
            data = refpack(data, int.from_bytes(data[2:5], 'big'))
        if entry.path.startswith('data/content/recipe/marquee/') and entry.path.endswith('.xml'):
            data = recipe(data)
        return data


def recipe(xml):
    """Skate 2 lists a material-only <mod> (no LODs) before each slot's model;
    Skate 3 recipes have just the model, which the builder expects."""
    import xml.etree.ElementTree as ET
    root = ET.fromstring(xml)
    for component in root.findall('comp'):
        for mod in component.findall('mod'):
            if mod.find('lod') is None:
                component.remove(mod)
    return ET.tostring(root)


def characters(archive):
    names = sorted({Path(e.path).stem for e in archive.entries
                    if e.path.startswith('data/content/recipe/marquee/') and e.path.endswith('.xml')})
    result = []
    for name in names:
        if name.startswith(SKIP_PREFIXES) or name.endswith(SKIP_SUFFIXES):
            continue
        base = name
        result.append({'key': 's2_' + base, 'recipe': name,
                       'name': LABELS.get(base, base.replace('_', ' ').title()),
                       'category': 'Special' if base in SPECIAL else 'Pro',
                       'animation_style': native_roster.STYLES.get(base, 'Aggressive')})
    return result


def native_directory(installation):
    current = json.loads((installation / 'assets/private/customisation/current.json').read_text())
    return installation / 'assets/private/customisation/sets' / current['set'] / 'native-roster'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--skate2', type=Path, required=True)
    parser.add_argument('--installation', type=Path, required=True)
    parser.add_argument('--work', type=Path, default=Path.home() / 'Downloads/SkateDLC/work/roster')
    parser.add_argument('--only', nargs='*')
    args = parser.parse_args()
    archive = Archive(args.skate2 / 'data/content/marquee.big')
    roster = characters(archive)
    if args.only:
        roster = [r for r in roster if r['key'] in args.only or r['recipe'] in args.only]
    print(f'Skate 2 roster: {len(roster)} characters', flush=True)
    # Point the Skate 3 builder at this archive and roster.
    native_roster.BigArchive = lambda _path: archive
    native_roster.roster = lambda _rows: roster
    collections = args.work / 'collections.json'
    args.work.mkdir(parents=True, exist_ok=True)
    collections.write_text('{"collections": []}')
    library = native_directory(args.installation)
    library.mkdir(parents=True, exist_ok=True)
    report = native_roster.prepare(args.skate2, args.installation / 'assets', library, collections, args.work)
    for item in report:
        if item['status'] != 'ready':
            continue
        manifest = library / 'entries' / item['id'] / 'manifest.json'
        data = json.loads(manifest.read_text())
        if data.get('game') != 'Skate 2':
            data['game'] = 'Skate 2'
            manifest.write_text(json.dumps(data, indent=2))
    ready = sum(r['status'] == 'ready' for r in report)
    print(f'Skate 2 roster ready: {ready}/{len(report)} in {library}')
    for r in report:
        if r['status'] != 'ready':
            print(' ', r['status'], r['key'], r.get('error', ''))


if __name__ == '__main__':
    main()
