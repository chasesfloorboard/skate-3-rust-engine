"""Challenge-map assets for the teleport screen, from the disc front end.

Output: <installation>/assets/private/ui/map/{map.png, locations/<name>.png,
map.json}. map.json maps each teleport destination id to its location photo,
retail title and description, and (for the three city districts) a position on
map.png in 0..1 texture coordinates.

Positions: each district's `world` VLT record carries a 2D offset that places
its local coordinates in one city frame (Vector2 field B88A4034D63E905E). The
city frame -> map1024 projection below was fitted by matching the teleport
clusters of Downtown, University and Industrial to where those districts are
drawn on the map (centre error <= 23 px of 1024); treat markers as approximate.

Usage:
  python tools/prepare_map_ui.py --game-root DISC --installation INSTALL_DIR
"""
import argparse
import json
import shutil
import struct
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent / 'vendor'))
from skate3_ui.language import parse_language_table  # noqa: E402
from skate3_ui.project import extract_project  # noqa: E402

# map1024 pixel = K * city + B, per axis (city x -> u, city z -> v).
PROJECTION = {'u': (0.2098, 396.4), 'v': (0.1781, 469.1)}
MAP_SIZE = 1024.0
OFFSET_FIELD = 'Hash_B88A4034D63E905E'
WORLD_NAME_FIELD = 'Hash_2A655E8A29C1F642'
TITLE_FIELD = 'Hash_174B301910A902C9'
DESCRIPTION_FIELD = 'Hash_6B0BD2BAE508B742'
IMAGE_FIELD = 'Hash_786A2D24F475342B'
PREFIXES = ('data/fe/source/images/map/map1024', 'data/fe/source/images/locations/',
            'data/fe/source/images/icons/mini_teleport', 'data/fe/source/screens/map/challengemap',
            'data/fe/languages/english/', 'data/fe/languages/labels/')
# Retail "Locations" groups: (title label, helper-text label, teleport maps).
GROUPS = (
    ('ID_DISTRICT_DOWNTOWN', 'ID_MAP_HELPER_LOCATION_DOWNTOWN', ('DownTown',)),
    ('ID_DISTRICT_INDUSTRY', 'ID_MAP_HELPER_LOCATION_INDUSTRIAL', ('Industrial',)),
    ('ID_DISTRICT_UNIVERSITY', 'ID_MAP_HELPER_LOCATION_UNIVERSITY', ('University',)),
    ('skate.School', 'ID_MAP_HELPER_LOCATION_SKATE_SCHOOL', ('SkateSchool',)),
    ('skate.Park', 'ID_MAP_HELPER_LOCATION_SKATE_PARKS', ('StartPark', 'MegaPark', 'DownTownSkatePark', 'IndustrialSkatePark')),
    ('Maloof Money Cup', 'ID_MAP_HELPER_LOCATION_MALOOF', ('MaloofMoneyCup',)),
    ('Black Box Park', 'ID_MAP_HELPER_LOCATION_BLACKBOXPARK', ('BlackBoxPark',)),
)


# Private code points in the retail strings (the front-end font draws the
# skate. logo and typographic punctuation at these slots).
GLYPHS = {'\x81': "'", '\x82': '', '\x89': ' - ', '«': 'skate.'}


def clean(text):
    for code, replacement in GLYPHS.items():
        text = text.replace(code, replacement)
    return ' '.join(text.split())


def inherited(records, key, field):
    while key in records:
        record = records[key]
        if field in record['fields']:
            return record['fields'][field]['data']
        key = record.get('parent')
    return None


def prepare(game_root, installation, report=print):
    installation = Path(installation)
    private = installation / 'assets/private'
    destinations = json.loads((private / 'teleports.json').read_text(encoding='utf-8'))['destinations']
    database = next(private.glob('customisation/sets/*/database/collections.json'), None)
    if database is None:
        raise RuntimeError('Character database export is missing')
    collections = json.loads(database.read_text(encoding='utf-8'))['collections']
    worlds = {r['key']: r for r in collections if r['class'] == 'world'}
    locations = {r['key']: r for r in collections if r['class'] == 'fe_locations'}
    offsets = {}
    for key in worlds:
        name, offset = inherited(worlds, key, WORLD_NAME_FIELD), inherited(worlds, key, OFFSET_FIELD)
        if name and offset and name.startswith('DIST_'):
            offsets.setdefault(name[5:].lower(), struct.unpack('>ff', bytes.fromhex(offset)))
    output = private / 'ui/map'
    with tempfile.TemporaryDirectory() as temp:
        cache = Path(temp) / 'ui'
        extract_project(Path(game_root), cache, include_dynamic=True, prefixes=PREFIXES, force=True)
        fe = cache / 'assets/data/fe/source/images'
        languages = cache / 'raw/data/fe/languages'
        english = parse_language_table(next(languages.glob('english/*Global*.BIN')))
        labels = parse_language_table(next(languages.glob('labels/*Global*.BIN')))
        text = dict(zip(labels.strings, english.strings))
        (output / 'locations').mkdir(parents=True, exist_ok=True)
        shutil.copyfile(next((fe / 'map/map1024').glob('*.png')), output / 'map.png')
        # Target-ring marker and the blue arc drawn (four times) around the selection.
        shutil.copyfile(next((fe / 'icons/mini_teleport').glob('*.png')), output / 'marker.png')
        screen = cache / 'assets/data/fe/source/screens/map/challengemap'
        shutil.copyfile(next(screen.glob('0014_*.png')), output / 'ring.png')
        entries = {}
        for d in destinations:
            entry = {'title': d['name']}
            location = locations.get(d['id'])
            if location:
                image = inherited(locations, d['id'], IMAGE_FIELD)
                stem = image.replace('\\', '/').rsplit('/', 1)[-1] if image else None
                source = next((fe / 'locations' / stem).glob('*.png'), None) if stem else None
                if source and stem != 'locked':
                    shutil.copyfile(source, output / 'locations' / f'{stem}.png')
                    entry['image'] = f'locations/{stem}.png'
                title = text.get(inherited(locations, d['id'], TITLE_FIELD) or '')
                description = text.get(inherited(locations, d['id'], DESCRIPTION_FIELD) or '')
                if title:
                    entry['title'] = clean(title)
                if description:
                    entry['description'] = clean(description)
            offset = offsets.get(d['map'].lower())
            if offset and d.get('matrix') and d['map'].lower() in ('university', 'downtown', 'industrial'):
                x, z = d['matrix'][3][0] + offset[0], d['matrix'][3][2] + offset[1]
                ku, bu = PROJECTION['u']
                kv, bv = PROJECTION['v']
                entry['position'] = [round((ku * x + bu) / MAP_SIZE, 4), round((kv * z + bv) / MAP_SIZE, 4)]
            entries[d['id']] = entry
    groups = []
    for title, helper, maps in GROUPS:
        members = [d['id'] for d in destinations if d['map'] in maps and d.get('matrix')]
        if members:
            groups.append({'title': clean(text.get(title, title)), 'description': clean(text.get(helper, '')),
                           'destinations': sorted(members, key=lambda i: entries[i]['title'].lower())})
    (output / 'map.json').write_text(json.dumps({'image': 'map.png', 'marker': 'marker.png', 'ring': 'ring.png',
                                                 'groups': groups, 'destinations': entries}, indent=1),
                                     encoding='utf-8')
    placed = sum('position' in e for e in entries.values())
    pictured = sum('image' in e for e in entries.values())
    report(f'Challenge map: {placed} placed, {pictured} with photos')
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--game-root', type=Path, required=True)
    parser.add_argument('--installation', type=Path, required=True)
    args = parser.parse_args()
    print(f'Challenge map ready: {prepare(args.game_root, args.installation)}')


if __name__ == '__main__':
    main()
