"""Skate 2 physics tuning for the Skate 2 edition and Freeskate's physics toggle.

Skate 2's data/db/db.big holds one skater vault (skater.bin/.vlt) in the same
AttribSys format as Skate 3's schema + collections pair. The result starts
from the installed Skate 3 collections and takes, for every physics_* record
both games share, each field Skate 2 also has. Skate 3 moved some of Skate
2's base values (jump heights, push strength, pumping, auto body spin) into
per-difficulty physics_mode records; its "default" mode holds exactly Skate
2's numbers, so every difficulty gets that record (Skate 2 had no difficulty).
Camera, animation and front-end records stay Skate 3's.

  python -m tools.skate2.physics --skate2 ~/Downloads/"Skate 2" --installation <install>
"""
import argparse
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from asset_pipeline.vlt import convert  # noqa: E402
from skate2 import bigf  # noqa: E402

OUTPUT = 'skater-collections-skate2.json'
# Difficulty records that pick up Skate 2's (= Skate 3 "default") values.
DIFFICULTIES = ('easy', 'normal', 'hardcore')


def skate2_vault(disc, work):
    database = disc / 'data/db/db.big'
    bigf.extract(database, work)
    stem = Path(work) / 'data/db/skater'
    names = (Path(__file__).resolve().parents[1] / 'asset_pipeline/names.txt').read_text(encoding='utf-8').splitlines()
    return convert(stem, stem, names)['collections']


def field_from(skate3, skate2):
    """Skate 2's value in Skate 3's encoding, or None when they don't line up."""
    data = skate2['data']
    if len(data) != len(skate3['data']):
        # Skate 2 stores Bool as 4 bytes, Skate 3 as one.
        if skate3['type'].endswith('Bool') and skate2['type'].endswith('Bool'):
            data = data[:len(skate3['data'])]
        else:
            return None
    field = dict(skate3, data=data)
    if 'array' in skate3 and 'array' in skate2:
        field['array'] = skate2['array']
    return field


def merge(skate3_rows, skate2_rows):
    by_key = {(r['class'], r['key']): r for r in skate2_rows}
    merged, taken, skipped = [], 0, []
    for row in skate3_rows:
        row = dict(row, fields=dict(row['fields']))
        other = by_key.get((row['class'], row['key']))
        if row['class'].startswith('physics') and row['class'] != 'physics_mode' and other:
            for name, field in row['fields'].items():
                if name in other['fields']:
                    value = field_from(field, other['fields'][name])
                    if value is None:
                        skipped.append(f"{row['class']}/{name}")
                    else:
                        taken += value['data'] != field['data']
                        row['fields'][name] = value
            row['source'] = 'skate2 skater.vlt'
        merged.append(row)
    modes = {r['key']: r for r in merged if r['class'] == 'physics_mode'}
    default = modes['default']['fields']
    for key in DIFFICULTIES:
        modes[key]['fields'] = dict(default)
        modes[key]['source'] = 'skate2 (skate3 physics_mode default)'
    return merged, taken, skipped


def build(disc, installation, report=print):
    stock = Path(installation) / 'assets/private/stock'
    skate3 = json.loads((stock / 'skater-collections.json').read_text(encoding='utf-8'))
    with tempfile.TemporaryDirectory() as work:
        skate2 = skate2_vault(Path(disc), work)
    rows, taken, skipped = merge(skate3['collections'], skate2)
    (stock / OUTPUT).write_text(json.dumps(dict(skate3, collections=rows)), encoding='utf-8')
    report(f'Skate 2 physics: {taken} values differ from Skate 3; {len(skipped)} fields kept ({", ".join(skipped) or "none"})')
    return stock / OUTPUT


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--skate2', type=Path, required=True, help='Extracted Skate 2 disc')
    parser.add_argument('--installation', type=Path, required=True)
    args = parser.parse_args()
    print(build(args.skate2.expanduser(), args.installation.expanduser()))


if __name__ == '__main__':
    main()
