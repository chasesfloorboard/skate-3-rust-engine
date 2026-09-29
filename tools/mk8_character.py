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


def centre_rings(path):
    """Put each ring joint (JIGGLE_SPIN_) at its ring's centre, with its Y
    axis along the arm through it, so jiggle.rs can hang the ring on the arm
    and spin it about its own centre. The rips hang the ring off-centre by
    more than it clears the arm; the ring moves in so it can hang any way
    without cutting the arm."""
    import pygltflib
    g = pygltflib.GLTF2().load(str(path))
    blob = bytearray(g.binary_blob())

    def acc(i):
        a = g.accessors[i]
        v = g.bufferViews[a.bufferView]
        n = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}[a.type]
        dt = {5126: np.float32, 5123: np.uint16, 5125: np.uint32, 5121: np.uint8}[a.componentType]
        start = (v.byteOffset or 0) + (a.byteOffset or 0)
        return np.frombuffer(bytes(blob), dt, a.count * n, start).reshape(a.count, n).copy(), start

    skin = g.skins[0]
    names = [g.nodes[j].name for j in skin.joints]
    binds, binds_at = acc(skin.inverseBindMatrices)
    world = [np.linalg.inv(b.reshape(4, 4).T) for b in binds]
    parent = {c: i for i, n in enumerate(g.nodes) for c in (n.children or [])}
    joint_of = {node: i for i, node in enumerate(skin.joints)}
    prims = [p for n in g.nodes if n.mesh is not None for p in g.meshes[n.mesh].primitives]
    for ring, name in enumerate(names):
        if not name.startswith('JIGGLE_SPIN_'):
            continue
        hand = joint_of[parent[skin.joints[ring]]]
        arm = joint_of.get(parent.get(skin.joints[hand]))
        if arm is None:
            continue
        origin = world[hand][:3, 3]
        axis = origin - world[arm][:3, 3]
        axis /= np.linalg.norm(axis)
        ring_points, arm_points = [], []
        for p in prims:
            pos, _ = acc(p.attributes.POSITION)
            jj, _ = acc(p.attributes.JOINTS_0)
            ww, _ = acc(p.attributes.WEIGHTS_0)
            share = lambda js: (ww * np.isin(jj, js)).sum(1)
            ring_points.append(pos[share([ring]) > 0.5])
            arm_points.append(pos[share([hand, arm]) > 0.5])
        points = np.concatenate(ring_points)
        if len(points) < 8:
            continue
        centre = points.mean(0)
        along = (points - centre) @ axis
        inner = np.linalg.norm(points - centre - np.outer(along, axis), axis=1).min()
        t = (centre - origin) @ axis
        hang = centre - origin - t * axis
        # The arm's widest point within the ring's width.
        limbs = np.concatenate(arm_points)
        lt = (limbs - origin) @ axis
        slab = limbs[np.abs(lt - t) < np.ptp(along) / 2 + 0.01]
        widest = np.linalg.norm(slab - origin - np.outer((slab - origin) @ axis, axis), axis=1).max() if len(slab) else 0.0
        room = max(inner - widest - 0.004, 0.0)
        length = np.linalg.norm(hang)
        new_centre = origin + t * axis + (hang / length * min(length, room) if length > 1e-6 else 0)
        shift = new_centre - centre
        for p in prims:
            pos, at = acc(p.attributes.POSITION)
            jj, _ = acc(p.attributes.JOINTS_0)
            ww, _ = acc(p.attributes.WEIGHTS_0)
            mine = (ww * (jj == ring)).sum(1) > 0.5
            if mine.any():
                pos[mine] += shift
                blob[at:at + pos.nbytes] = pos.astype(np.float32).tobytes()
                a = g.accessors[p.attributes.POSITION]
                a.min, a.max = pos.min(0).tolist(), pos.max(0).tolist()
        # Joint frame: Y along the arm, X toward the hang, at the centre.
        x = hang - (hang @ axis) * axis
        x = x / np.linalg.norm(x) if np.linalg.norm(x) > 1e-6 else np.cross(axis, [0, 0, 1])
        frame = np.eye(4)
        frame[:3, 0], frame[:3, 1], frame[:3, 2] = x, axis, np.cross(x, axis)
        frame[:3, 3] = new_centre
        world[ring] = frame
        binds[ring] = np.linalg.inv(frame).T.reshape(-1)
        local = np.linalg.inv(world[hand]) @ frame
        g.nodes[skin.joints[ring]].matrix = local.T.reshape(-1).tolist()
        print(f'{name}: centred, hangs {min(length, room) * 100:.1f} cm (ring inner {inner * 100:.1f} cm, arm {widest * 100:.1f} cm)')
    blob[binds_at:binds_at + binds.nbytes] = binds.astype(np.float32).tobytes()
    g.set_binary_blob(bytes(blob))
    g.save_binary(str(path))



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
            subprocess.run([sys.executable, str(HERE / 'mk8_to_mixamo.py'), str(args.model), str(mixamo)], check=True)
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
        # Mario Kart 8 caps stay on in a bail; Odyssey caps come off.
        mk8_convert.KEEP = set() if args.model.suffix.lower() == '.smd' else {'cap'}
        report = convert_glb(mixamo, reference, staging / 'character.glb', include_board=False, keep_height=args.height)
        if mk8_convert.SPIN:
            centre_rings(staging / 'character.glb')
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
