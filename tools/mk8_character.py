"""Add a Mario Kart 8 racer (Collada rip) to the game's custom character
library as a skater that keeps its own proportions.

mk8_to_mixamo.py rebuilds the racer with Mixamo bone names; the character
importer's keep-proportions mode then puts the stock skeleton's joints at the
racer's joint positions (stock orientations), and the manifest's
"proportions" tells the game to play stock animation rotations on the
racer's own bone lengths, lowering the hips by the leg-length ratio.

Usage:
  python tools/mk8_character.py RACER.dae NAME [--height 1.35] [--library DIR]
"""
import argparse

import numpy as np
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / 'mixamo_to_skate'))
sys.path.insert(0, str(HERE))
from mk8_convert import convert_glb, sha  # noqa: E402
from library_import import thumbnail  # noqa: E402


# Standing heights (m) by Mario Kart 8 weight class: rips use mixed units, so
# sizes are set rather than taken from the files. Babies are Toadette-sized.
HEIGHTS = {
    **{n: 0.95 for n in ('Baby Mario', 'Baby Luigi', 'Baby Peach', 'Baby Daisy', 'Baby Rosalina',
                         'Toad', 'Toadette', 'Lemmy')},
    **{n: 1.0 for n in ('Koopa Troopa', 'Lakitu', 'Shy Guy', 'Larry', 'Wendy', 'Isabelle')},
    'Iggy': 1.1, 'Villagers': 1.2,
    **{n: 1.35 for n in ('Mario', 'Metal Mario', 'Tanooki Mario', 'Yoshi', 'Ludwig', 'Morton', 'Roy')},
    **{n: 1.45 for n in ('Luigi', 'Peach', 'Peach (Suit)', 'Pink Gold Peach', 'Pink Gold Peach (Suit)',
                         'Daisy', 'Daisy (Suit)', 'Cat Peach', 'Mii', 'Wario')},
    **{n: 1.75 for n in ('Rosalina', 'Rosalina (Suit)', 'Link', 'Donkey Kong')},
    **{n: 1.9 for n in ('Bowser', 'Dry Bowser', 'Waluigi')},
}


def soles(path):
    """(lowest foot-skinned vertex, lowest vertex, foot joint height) of a
    converted character.glb in bind pose. A tail or shell can hang lower
    than the feet, which lifted Dry Bowser off the board when the ankle
    height was measured from the lowest point."""
    import pygltflib
    g = pygltflib.GLTF2().load(str(path))
    blob = g.binary_blob()

    def acc(i):
        a = g.accessors[i]
        v = g.bufferViews[a.bufferView]
        n = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}[a.type]
        dt = {5126: np.float32, 5123: np.uint16, 5125: np.uint32, 5121: np.uint8}[a.componentType]
        return np.frombuffer(blob, dt, a.count * n, (v.byteOffset or 0) + (a.byteOffset or 0)).reshape(a.count, n)

    skin = g.skins[0]
    names = [g.nodes[j].name.upper() for j in skin.joints]
    feet = {i for i, n in enumerate(names) if ('FOOT' in n or 'TOE' in n) and 'REPARENTED' not in n}
    binds = acc(skin.inverseBindMatrices).reshape(-1, 4, 4)
    foot_y = float(np.mean([np.linalg.inv(binds[i].T)[1, 3] for i, n in enumerate(names) if n in ('LEFTFOOT', 'RIGHTFOOT')]))
    sole, low = [], []
    for node in g.nodes:
        if node.mesh is None:
            continue
        for p in g.meshes[node.mesh].primitives:
            pos = acc(p.attributes.POSITION)
            joints, weights = acc(p.attributes.JOINTS_0), acc(p.attributes.WEIGHTS_0)
            low.append(float(pos[:, 1].min()))
            on_feet = np.array([sum(w for j, w in zip(jj, ww) if j in feet) for jj, ww in zip(joints, weights)]) > 0.5
            if on_feet.any():
                sole.append(float(pos[on_feet, 1].min()))
    return (min(sole) if sole else None), min(low), foot_y


# Joints that spin freely (as rings) rather than swing, per racer.
SPIN = {'Wendy': ('seleeve',)}

# Hair under the cap for capped racers other than Mario (bald, with a comb-over).
HAIR = {'Luigi': '3a2010', 'Wario': '2e1a0e', 'Waluigi': '241a26'}


def default_library():
    data = os.environ.get('XDG_DATA_HOME') or str(Path.home() / '.local/share')
    return Path(data) / 'Skate3RustEngine/custom-characters'


def default_reference():
    found = sorted((Path.home() / 'Games/Skate3Rust/data/installations').glob('*/assets/private/skater.glb'))
    if not found:
        raise SystemExit('No installed skater.glb found; pass --reference')
    return found[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('model', type=Path)
    parser.add_argument('name')
    parser.add_argument('--height', type=float, default=None, help='standing height in metres')
    parser.add_argument('--relative', type=float, default=1.17,
                        help="without --height: scale of the model's own MK8 size (1.17 makes Mario 1.35 m, babies small)")
    parser.add_argument('--library', type=Path, default=None)
    parser.add_argument('--game', default='Mario Kart 8', help='game shown in the customiser Model page')
    parser.add_argument('--reference', type=Path, default=None)
    args = parser.parse_args()
    library = args.library or default_library()
    reference = args.reference or default_reference()
    entries = library / 'entries'
    entries.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.mk8-', dir=library) as temp:
        temp = Path(temp)
        mixamo = temp / 'source.glb'
        hair = ['--hair', HAIR[args.name]] if args.name in HAIR else []
        if args.model.suffix.lower() == '.smd':
            # Odyssey rips (tools/smd_to_mixamo.py): body plus the first hand
            # pose and eyebrows (eyelids are blink shapes).
            # Odyssey Mario outfits: the cap with the hair tucked under it
            # (<stem>_Cap), else the loose hair (Mario_Hair).
            stem, folder = args.model.stem, args.model.parent
            find = lambda name: next(iter(folder.glob(f'{name}.[Ss][Mm][Dd]')), None)
            parts = [p for suffix in ('_LHand1', '_RHand1', '_Eyebrow1') for p in [find(stem + suffix)] if p]
            cap = find(stem + '_Cap')
            parts += [cap] if cap else [p for p in [find(stem + '_Hair') or find('Mario_Hair')] if p]
            subprocess.run([sys.executable, str(HERE / 'smd_to_mixamo.py'), str(args.model), str(mixamo),
                            '--parts', *map(str, parts)], check=True)
        else:
            subprocess.run([sys.executable, str(HERE / 'mk8_to_mixamo.py'), str(args.model), str(mixamo), *hair], check=True)
        if args.height is None:
            args.height = HEIGHTS.get(args.name, HEIGHTS.get(args.name.split(' (')[0]))
        if args.height is None:
            # MK8 racers share one scale: keep their relative sizes.
            from glb import Document
            from mk8_convert import mesh_bounds
            points = mesh_bounds(Document(mixamo))
            args.height = float(np.ptp(points[:, 1]))*args.relative
        digest = hashlib.sha256((sha(mixamo) + sha(reference) + f'keep-{args.height}' + 'library-v1' + 'jiggle-v1').encode()).hexdigest()
        target = entries / digest
        if target.exists():
            print(f'{args.name}: already in the library ({digest})')
            return
        staging = temp / 'entry'
        staging.mkdir()
        # No board: the player's own customised board stays under the character.
        import mk8_convert
        mk8_convert.SPIN = set(SPIN.get(args.name, ()))
        report = convert_glb(mixamo, reference, staging / 'character.glb', include_board=False, keep_height=args.height)
        sole, _, foot_y = soles(staging / 'character.glb')
        if sole is not None:
            report['ankle'] = float(foot_y - sole)
        (staging / 'report.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
        mixamo.replace(staging / 'source.glb')
        thumbnail(staging / 'character.glb', staging / 'preview.png')
        manifest = {'version': 1, 'id': digest, 'name': args.name[:64], 'game': args.game,
                    'source_sha256': sha(staging / 'source.glb'), 'reference_sha256': sha(reference),
                    'created_utc': datetime.now(timezone.utc).isoformat(),
                    'proportions': {'hips_ratio': report['hips_ratio'], 'leg_ratio': report['leg_ratio'],
                                    'ankle': report['ankle'], 'stock_ankle': report['stock_ankle']}}
        (staging / 'manifest.json').write_text(json.dumps(manifest, indent=2), encoding='utf-8')
        staging.rename(target)
    print(f'{args.name}: added ({digest}), hips ratio {report["hips_ratio"]:.2f}')


if __name__ == '__main__':
    main()
