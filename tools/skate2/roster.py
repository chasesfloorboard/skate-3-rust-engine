"""Skate 2 pros and specials as native characters (Call Skater "Skate 2" sections).

Skate 2's data/content/marquee.big has Skate 3's Marquee layout (recipe XML,
per-slot rx2 models, rx2 textures) inside a classic BIGF archive whose files
are RefPack-compressed. This adapts the archive and runs the Skate 3 roster
builder (asset_pipeline/native_roster.py, left untouched because it is part of
the customiser fingerprint) on it. Entries get "s2_" keys, so they never share
an identity with the Skate 3 versions of the same pro, and "game": "Skate 2".

  python -m tools.skate2.roster --skate2 ~/Downloads/"Skate 2" --installation <install> [--only big_black]

Output: <install>/assets/private/skate2/native-roster (setup's "skate2" group).
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

    def __init__(self, path, shared=None):
        self.path = Path(path)
        self.entries = [Entry(name, offset, size) for name, offset, size in bigf.entries(self.path)]
        self.sources = {e.path: self.path for e in self.entries}
        # Some pros use stock create-a-skater textures, which Skate 2 keeps
        # only in createacharacter.big: offer them under marquee/texture too.
        if shared and Path(shared).exists():
            known = set(self.sources)
            for name, offset, size in bigf.entries(shared):
                if '/createacharacter/texture/' in name:
                    alias = 'data/content/marquee/texture/' + name.rsplit('/', 1)[1]
                    if alias not in known:
                        self.entries.append(Entry(alias, offset, size))
                        self.sources[alias] = Path(shared)
                        known.add(alias)

    def read(self, entry):
        with self.sources[entry.path].open('rb') as f:
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
        models = []
        for mod in component.findall('mod'):
            if mod.find('lod') is None:
                component.remove(mod)
            else:
                models.append(mod)
        # A second model in one slot (Terry Kennedy's two wrist items): the
        # builder takes one per slot, so keep the first.
        for mod in models[1:]:
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


def prepare(skate2, installation, work, report=print, only=None, library=None):
    """Build the roster into <installation>/assets/private/skate2/native-roster
    (setup's Skate 2 group owns it; the customiser set may be rebuilt)."""
    archive = Archive(Path(skate2) / 'data/content/marquee.big', Path(skate2) / 'data/content/createacharacter.big')
    roster = characters(archive)
    if only:
        roster = [r for r in roster if r['key'] in only or r['recipe'] in only]
    report(f'Skate 2 characters: converting {len(roster)}')
    # Point the Skate 3 builder at this archive and roster.
    native_roster.BigArchive = lambda _path: archive
    native_roster.roster = lambda _rows: roster
    work = Path(work)
    work.mkdir(parents=True, exist_ok=True)
    collections = work / 'collections.json'
    collections.write_text('{"collections": []}')
    library = Path(library) if library else Path(installation) / 'assets/private/skate2/native-roster'
    library.mkdir(parents=True, exist_ok=True)
    result = native_roster.prepare(Path(skate2), Path(installation) / 'assets', library, collections, work)
    for item in result:
        if item['status'] != 'ready':
            continue
        manifest = library / 'entries' / item['id'] / 'manifest.json'
        data = json.loads(manifest.read_text())
        if data.get('game') != 'Skate 2':
            data['game'] = 'Skate 2'
            manifest.write_text(json.dumps(data, indent=2))
    ready = sum(r['status'] == 'ready' for r in result)
    (library / 'complete.json').write_text(json.dumps({'characters': ready, 'unavailable': [
        {'key': r['key'], 'error': r.get('error', '')} for r in result if r['status'] != 'ready']}))
    report(f'Skate 2 characters: {ready}/{len(result)} ready')
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--skate2', type=Path, required=True)
    parser.add_argument('--installation', type=Path, required=True)
    parser.add_argument('--work', type=Path, default=Path.home() / '.cache/skate3rust-skate2/roster')
    parser.add_argument('--only', nargs='*')
    args = parser.parse_args()
    result = prepare(args.skate2, args.installation, args.work, only=args.only)
    for r in result:
        if r['status'] != 'ready':
            print(' ', r['status'], r['key'], r.get('error', ''))


if __name__ == '__main__':
    main()
