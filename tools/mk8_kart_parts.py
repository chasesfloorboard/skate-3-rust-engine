"""Build kart parts for the Mario Kart mod from Mario Kart 8 rips.

Bodies (Karts/), tyres (Tires/, the kart tyre TireK_* of each pack) and
seated drivers (Racers/) become .glb files under the mod's parts/ folder, with
parts.json listing them for the mod menu:

- every body is sized and centred onto the mod's reference kart footprint
  (kart.glb's body) so it sits on the same four physics wheels;
- tyres are centred on their axle; their radius is recorded so the game scales
  them to each wheel;
- the driver is the player's own character (custom models ride too).

Usage:
  python tools/mk8_kart_parts.py MK8_DIR MOD_DIR
    MK8_DIR  folder with extracted_karts/, extracted_tires/, extracted/
    MOD_DIR  unpacked Mario Kart mod (kart.glb, vehicle.json, ...)
"""
import argparse
import json
import re
import sys
from pathlib import Path

import collada
import numpy as np
import trimesh

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mk8_to_glb import SEATED, Y_UP, Rig, primitives, write_glb  # noqa: E402

# Grip per tyre family (the mod's dry setup is 1.3), after MK8's traction
# stats: slicks and slims hug the road, metal and wood slide.
# Tyre families drawn a little smaller than the physics wheel (their MK8
# look is low profile); the hub drops so they still meet the ground.
SMALLER = ('Standard', 'Blue Standard', 'Monster', 'Hot Monster', 'Slick', 'Cyber Slick')
GRIP = {'Standard': 1.3, 'Monster': 1.25, 'Slick': 1.5, 'Slim': 1.4, 'Metal': 1.05,
        'Wood': 1.15, 'Leaf': 1.2}
TURN = 1
IGNORE = [collada.common.DaeUnsupportedError, collada.common.DaeBrokenRefError]


def load(path):
    return collada.Collada(str(path), ignore=IGNORE)


def albedo(folder: Path, texture: Path):
    """The exporter's placeholder albedo replaced by the Mario colour sheet
    (else the first real albedo in the folder)."""
    if texture is not None and texture.is_file() and 'dummy' not in texture.name.lower():
        return texture
    candidates = [p for p in folder.rglob('*') if p.suffix.lower() == '.png' and '_alb' in p.name.lower()
                  and 'dummy' not in p.name.lower()]
    # Body sheets first (arm/strut sheets sort ahead of them otherwise).
    candidates.sort(key=lambda p: (not p.name.lower().startswith('body'), p.name.lower()))
    for key in ('_mro_', '_mro', '_a_alb'):
        for p in candidates:
            if key in p.name.lower():
                return p
    return candidates[0] if candidates else texture


def parts_of(dae: Path, scale=1.0, rig=None):
    document = load(dae)
    result = []
    for name, texture, positions, normals, uvs in primitives(document, dae.parent, 'A', rig):
        # Lightmap and emblem passes duplicate the geometry (layers named _2,
        # _3 in some rips) and drew over the albedo.
        if texture is not None and (texture.name.lower().startswith('bake_') or 'emblem' in texture.name.lower()):
            continue
        result.append((name, albedo(dae.parent, texture), positions, normals, uvs))
    if rig is not None:
        hip = (Y_UP @ rig.world('Hip_1'))[:3, 3]
        result = [(n, t, p - hip, nm, uv) for n, t, p, nm, uv in result]
    return result


def body_styles(folder: Path, parts):
    """Colour styles for a body: its default albedo first, then the other
    sheets of the same body (bodyk_std_lig_alb, BodyK_Skl_B_Alb, ...)."""
    textures = [p[1] for p in parts if p[1] is not None and p[1].name.lower().startswith('body')
                and '_alb' in p[1].name.lower()]
    if not textures:
        return None, []
    default = textures[0]
    prefix = '_'.join(default.name.lower().split('_')[:2]) + '_'
    seen, styles = {default.name.lower()}, [default]
    for candidate in sorted(folder.rglob('*'), key=lambda p: p.name.lower()):
        name = candidate.name.lower()
        if (candidate.suffix.lower() == '.png' and name.startswith(prefix) and name.endswith('_alb.png')
                and 'dummy' not in name and name not in seen):
            seen.add(name)
            styles.append(candidate)
    return default, styles


def render_preview(parts, out: Path, width=256, height=192, yaw=-0.7, pitch=0.35):
    """A small textured three-quarter view of parts (for the mod menu)."""
    from PIL import Image
    cy, sy, cp, sp = np.cos(yaw), np.sin(yaw), np.cos(pitch), np.sin(pitch)
    view = np.array([[cy, 0, sy], [0, 1, 0], [-sy, 0, cy]])
    view = np.array([[1, 0, 0], [0, cp, -sp], [0, sp, cp]]) @ view
    points = np.concatenate([p[2] for p in parts]) @ view.T
    low, high = points.min(0), points.max(0)
    scale = min((width - 16) / max(high[0] - low[0], 1e-3), (height - 16) / max(high[1] - low[1], 1e-3))
    centre = (low + high) / 2
    rgb = np.zeros((height, width, 3)) + np.array([0.06, 0.08, 0.11])
    depth = np.full((height, width), -np.inf)
    light = np.array([0.3, 0.7, -0.6]); light /= np.linalg.norm(light)
    cache = {}
    for _, texture, positions, normals, uvs in parts:
        if texture is not None and texture not in cache:
            try:
                with Image.open(texture) as image:
                    image.thumbnail((512, 512))
                    cache[texture] = np.asarray(image.convert('RGB')) / 255.
            except Exception:
                cache[texture] = None
        tex = cache.get(texture)
        world = positions @ view.T
        screen = (world - centre) * scale
        screen[:, 0] += width / 2
        screen[:, 1] = height / 2 - screen[:, 1]
        shade = 0.75 + 0.6 * np.abs((normals @ view.T) @ light)
        for i in range(0, len(screen) - 2, 3):
            tri = screen[i:i + 3]
            x0, x1 = max(0, int(tri[:, 0].min())), min(width - 1, int(np.ceil(tri[:, 0].max())))
            y0, y1 = max(0, int(tri[:, 1].min())), min(height - 1, int(np.ceil(tri[:, 1].max())))
            if x0 > x1 or y0 > y1:
                continue
            (ax, ay), (bx, by), (cx, cy2) = tri[:, :2]
            den = (by - cy2) * (ax - cx) + (cx - bx) * (ay - cy2)
            if abs(den) < 1e-9:
                continue
            xs, ys = np.meshgrid(np.arange(x0, x1 + 1) + 0.5, np.arange(y0, y1 + 1) + 0.5)
            w0 = ((by - cy2) * (xs - cx) + (cx - bx) * (ys - cy2)) / den
            w1 = ((cy2 - ay) * (xs - cx) + (ax - cx) * (ys - cy2)) / den
            w2 = 1 - w0 - w1
            inside = (w0 >= 0) & (w1 >= 0) & (w2 >= 0)
            if not inside.any():
                continue
            z = w0 * tri[0, 2] + w1 * tri[1, 2] + w2 * tri[2, 2]
            region = depth[y0:y1 + 1, x0:x1 + 1]
            front = inside & (z > region)
            if not front.any():
                continue
            region[front] = z[front]
            if tex is not None:
                uv = w0[..., None] * uvs[i] + w1[..., None] * uvs[i + 1] + w2[..., None] * uvs[i + 2]
                u = (uv[..., 0] % 1.0) * (tex.shape[1] - 1)
                v = ((1.0 - uv[..., 1]) % 1.0) * (tex.shape[0] - 1)
                colour = tex[v.astype(int), u.astype(int)]
            else:
                colour = np.full(xs.shape + (3,), 0.7)
            s_ = w0 * shade[i] + w1 * shade[i + 1] + w2 * shade[i + 2]
            rgb[y0:y1 + 1, x0:x1 + 1][front] = (colour * s_[..., None])[front]
    Image.fromarray((np.clip(rgb, 0, 1) ** (1 / 1.25) * 255).astype(np.uint8)).save(out, optimize=True)


def cockpit(points, cell=0.04):
    """Surface heights under the default seat spot (|x| < 0.12 m, z -0.45..0.05):
    the top of the body per 4 cm column, as (25th percentile, median)."""
    inside = (np.abs(points[:, 0]) < 0.12) & (points[:, 2] > -0.45) & (points[:, 2] < 0.05)
    if not inside.any():
        return None
    tops = {}
    for key, y in zip(map(tuple, np.floor(points[inside][:, [0, 2]] / cell).astype(int)), points[inside][:, 1]):
        tops[key] = max(tops.get(key, -9.0), y)
    heights = np.array(list(tops.values()))
    return float(np.percentile(heights, 25)), float(np.median(heights))


def surface_points(parts, per_part=20000):
    """Points spread over the triangles (vertices alone miss large flat panels)."""
    out = []
    rng = np.random.default_rng(0)
    for _, _, positions, _, _ in parts:
        tri = positions.reshape(-1, 3, 3)
        if not len(tri):
            continue
        area = np.linalg.norm(np.cross(tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0]), axis=1) + 1e-12
        pick = rng.choice(len(tri), per_part, p=area / area.sum())
        a, b = rng.random((2, per_part, 1))
        flip = a + b > 1
        a[flip], b[flip] = 1 - a[flip], 1 - b[flip]
        t = tri[pick]
        out.append(t[:, 0] + a * (t[:, 1] - t[:, 0]) + b * (t[:, 2] - t[:, 0]))
    return np.concatenate(out)


# Hand-placed axles (m along the body, +Z forward) where the automatic
# placement misreads a bike, checked against its side view.
BIKE_WHEELS = {
    'City Tripper': {'front': 0.95},  # under the front fender, not past the leg shield
}

# Z-up, -Y-forward rips to the parts' Y-up, +Z-forward frame (bike joints).
JOINT_FRAME = np.array([[1, 0, 0], [0, 0, 1], [0, -1, 0]], dtype=float)


def bike_axles(folder: Path, turned: bool):
    """A bike's front and rear axles in its body's raw frame (as parts_of
    returns it, before placement), from its own suspension arms (ArmB_*M):
    the fork hangs from the steering head (Jnt_Handle) and ends at the front
    axle; the swing arm hangs from its pivot (Jnt_ArmB, else the body
    origin) and ends at the rear axle. Arms without a hanging fork are laid
    out from the body origin. None when the rip has no arm model."""
    import re as _re
    arm = next(iter(sorted(folder.rglob('ArmB_*M.dae'))), None)
    body = [p for p in folder.rglob('*.dae') if p.name.lower().startswith('bodyb') and 'fix' not in p.name.lower()]
    if arm is None or not body:
        return None
    document = load(max(body, key=lambda p: p.stat().st_size))
    joints = {}

    def walk(node, matrix):
        for child in node.children:
            if isinstance(child, collada.scene.Node):
                world = matrix @ child.matrix
                name = _re.sub(r'^(node-)?(Armature_)?', '', child.id or '')
                if name.startswith('Jnt_'):
                    joints[name] = world[:3, 3] @ JOINT_FRAME.T
                walk(child, world)
    for node in document.scene.nodes:
        walk(node, node.matrix)
    frame = (lambda v: v @ JOINT_FRAME.T) if turned else (lambda v: v)
    pieces = {_re.sub(r'^geom-', '', n).split('__')[0]: frame(p) for n, _, p, _, _ in parts_of(arm)}

    def far_end(points):
        distance = np.linalg.norm(points, axis=1)
        end = points[distance > distance.max() - 0.3].mean(0)
        end[0] = 0.0
        return end
    # Only a hanging fork pins the axles down; other arm sets are laid out
    # about the body origin, which says nothing about axle height.
    if 'ArmB_B' not in pieces or 'Sus' not in pieces or not {'Jnt_Handle', 'Jnt_ArmB'} <= joints.keys():
        return None
    fork = True
    front = (joints.get('Jnt_Handle', np.zeros(3)) if fork else np.zeros(3)) + far_end(pieces.get('Sus', pieces.get('ArmB_F')))
    rear = (joints.get('Jnt_ArmB', np.zeros(3)) if fork else np.zeros(3)) + far_end(pieces['ArmB_B'])
    # Back into the raw frame of the body geometry.
    if turned:
        front, rear = front @ JOINT_FRAME, rear @ JOINT_FRAME
    return front, rear


def bike_mounts(p, R, yc, half=0.05, cell=0.02, clip=0.03):
    """Front and rear axle z for a bike body (points: sampled surface, +Z
    forward): searching outward from the middle, the first spot where the
    tyre clears the body along its centre line (fork legs and fenders sit
    either side of the tyre, so only a thin slice counts)."""
    q = p[np.abs(p[:, 0]) < half]
    occ = set(map(tuple, np.floor(q[:, [2, 1]] / cell).astype(int)))
    key = lambda z, y: (int(np.floor(z / cell)), int(np.floor(y / cell)))
    disc = [(dz, dy) for dz in np.arange(-R, R + cell, cell) for dy in np.arange(-R, R + cell, cell) if np.hypot(dz, dy) < R * 0.95]

    def inside(z):
        return sum(key(z + dz, yc + dy) in occ for dz, dy in disc) / len(disc)

    result = []
    for sign in (1, -1):
        zs = np.arange(0.2, 1.6, 0.02) * sign
        result.append(float(next((z for z in zs if inside(z) <= clip), 0.9 * sign)))
    return result


def restyle(parts, old, new):
    return [(n, new if t == old else t, p, nm, uv) for n, t, p, nm, uv in parts]


def bounds(parts):
    points = np.concatenate([p[2] for p in parts])
    return points.min(0), points.max(0)


def transform(parts, scale, offset):
    return [(n, t, p * scale + offset, nm, uv) for n, t, p, nm, uv in parts]


def slug(name):
    return re.sub(r'[^a-z0-9]+', '-', name.lower()).strip('-')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('mk8', type=Path)
    parser.add_argument('mod', type=Path)
    args = parser.parse_args()
    out = args.mod / 'parts'
    for sub in ('bodies', 'tyres', 'drivers'):
        (out / sub).mkdir(parents=True, exist_ok=True)

    # Reference footprint: the mod kart's body meshes (not its wheel nodes).
    scene = trimesh.load(args.mod / 'kart.glb', force='scene')
    low, high = np.full(3, np.inf), np.full(3, -np.inf)
    for node in scene.graph.nodes_geometry:
        if any(node == 'kart_mesh_%d' % i for i in range(3, 9)) or any(node == 'kart_mesh_%d' % i for i in range(10, 16)):
            continue
        matrix, geometry = scene.graph[node]
        mesh = scene.geometry[geometry].copy()
        mesh.apply_transform(matrix)
        low, high = np.minimum(low, mesh.bounds[0]), np.maximum(high, mesh.bounds[1])
    ref_length, ref_centre, ref_bottom = high[2] - low[2], (low + high) / 2, low[1]
    # Menu preview of the mod's own kart, its textures saved beside it.
    original = []
    for node in scene.graph.nodes_geometry:
        matrix, geometry = scene.graph[node]
        mesh = scene.geometry[geometry].copy()
        mesh.apply_transform(matrix)
        uv = getattr(mesh.visual, 'uv', None)
        image = getattr(getattr(mesh.visual, 'material', None), 'baseColorTexture', None)
        texture = None
        if uv is not None and image is not None:
            texture = out / f'.original-{len(original)}.png'
            image.save(texture)
        faces = mesh.faces.reshape(-1)
        original.append((node, texture, mesh.vertices[faces], np.repeat(mesh.face_normals, 3, axis=0),
                         np.c_[uv[faces][:, 0], 1 - uv[faces][:, 1]] if uv is not None else np.zeros((len(faces), 2))))
    render_preview(original, out / 'bodies/original.png')
    original_floor = cockpit(surface_points(original))[0]
    vehicle = json.loads((args.mod / 'vehicle.json').read_text())
    seat = vehicle['seat']
    # Wheel centre height at rest and tyre radius (bike wheel gaps).
    axle_y = float(np.mean([w['position'][1] for w in vehicle['wheels']])) - vehicle['suspension_length']
    wheel_radius = float(np.mean([w['radius'] for w in vehicle['wheels']]))
    for p in out.glob('.original-*.png'):
        p.unlink()

    listing = {'bodies': [], 'tyres': [], 'drivers': []}
    bodies = {}
    for folder in sorted((args.mk8 / 'extracted_karts').iterdir()):
        daes = [p for p in folder.rglob('*.dae') if 'bodyk' in p.name.lower() or p.stem.lower() == folder.name.lower()]
        daes = daes or list(folder.rglob('*.dae'))
        # Prefer the original export over re-exports (BodyK_Pch_fix.dae merges its layers).
        daes = [p for p in daes if not any(k in p.stem.lower() for k in ('_fix', 'opencollada'))] or daes
        if daes:
            try:
                bodies[folder.name] = parts_of(max(daes, key=lambda p: p.stat().st_size))
            except Exception as error:
                print('SKIP body', folder.name, error)
    # Common MK8 scale: a typical body (median length; some rips use other
    # units) onto the reference footprint. Tyres and drivers share it.
    common = ref_length / float(np.median([np.ptp(np.concatenate([p[2] for p in b]), axis=0)[2] for b in bodies.values()]))
    print(f'MK8 scale factor {common:.4f}')

    for name, parts in bodies.items():
        folder = Path(name)
        try:
            # Some rips come out standing on their tail (taller than long):
            # turn them onto their wheels.
            turned = np.ptp(np.concatenate([p[2] for p in parts]), axis=0)[1] > np.ptp(np.concatenate([p[2] for p in parts]), axis=0)[2]
            turn = np.array([[1, 0, 0], [0, 0, TURN], [0, -TURN, 0]], dtype=float) if turned else np.eye(3)
            raw_axles = bike_axles(args.mk8 / 'extracted_karts' / name, turned)
            if turned:
                parts = [(n, t, p @ turn.T, nm @ turn.T, uv) for n, t, p, nm, uv in parts]
            b_low, b_high = bounds(parts)
            scale = ref_length / (b_high[2] - b_low[2])
            centre = (b_low + b_high) / 2 * scale
            offset = np.array([ref_centre[0] - centre[0], ref_bottom - b_low[1] * scale, ref_centre[2] - centre[2]])
            placed = transform(parts, scale, offset)
            daes = [d.name.lower() for d in (args.mk8 / 'extracted_karts' / name).rglob('*.dae')]
            bike = any(d.startswith('bodyb') for d in daes)
            mounts = None
            if bike and raw_axles is not None:
                # The bike's own suspension arms set its axles; the body is
                # raised so they sit at the physics wheels' height.
                front, rear = (a @ turn.T * scale + offset for a in raw_axles)
                lift = axle_y - (front[1] + rear[1]) / 2
                placed = transform(placed, 1.0, np.array([0.0, lift, 0.0]))
                mounts = [float(front[2]), float(rear[2])]
            elif bike:
                # No arm model: underside just below the axles, each tyre in
                # the body's wheel gap.
                lift = (axle_y - 0.1) - bounds(placed)[0][1]
                placed = transform(placed, 1.0, np.array([0.0, max(lift, 0.0), 0.0]))
                mounts = bike_mounts(surface_points(placed), wheel_radius, axle_y)
            if bike and name in BIKE_WHEELS:
                override = BIKE_WHEELS[name]
                mounts = [override.get('front', mounts[0]), override.get('rear', mounts[1])]
            default, styles = body_styles(args.mk8 / 'extracted_karts' / name, parts)
            files = []
            for index, style in enumerate(styles or [None]):
                file = f'parts/bodies/{slug(folder.name)}' + (f'-{index + 1}' if index else '') + '.glb'
                styled = restyle(placed, default, style) if index else placed
                write_glb(styled, args.mod / file, 1.0)
                render_preview(styled, args.mod / file.replace('.glb', '.png'))
                files.append(file)
            # Bikes (BodyB_*) straddle: two wheels, lean, seat above the saddle.
            # Karts keep the reference seat's height over their cockpit floor.
            # ATVs (BodyV_*) are straddled too, on four wheels.
            straddled = bike or any(d.startswith('bodyv') for d in daes)
            floor, saddle = cockpit(surface_points(placed)) or (original_floor, original_floor)
            height = saddle + 0.2 if straddled else seat[1] + floor - original_floor
            listing['bodies'].append({'name': folder.name, 'file': files[0], 'styles': files,
                                      'previews': [f.replace('.glb', '.png') for f in files],
                                      'layout': 'bike' if bike else 'atv' if straddled else 'kart',
                                      'seat': [seat[0], round(height, 3), seat[2]],
                                      **({'wheel_z': [round(z, 3) for z in mounts]} if mounts else {})})
            print('body', folder.name, len(files), 'styles')
        except Exception as error:
            print('SKIP body', folder.name, error)

    for folder in sorted((args.mk8 / 'extracted_tires').iterdir()):
        # "Standard & Blue Standard": the pack's first model and albedo are the
        # first name; the second model (TireK_Zbi) or albedo (Tire_Zst) the other.
        names = folder.name.replace(' Tires', '').split(' & ')
        daes = [p for p in sorted(folder.rglob('TireK_*.dae')) if 'opencollada' not in p.name.lower()]
        if not daes:
            continue
        family = next((k for k in GRIP if k.lower() in names[0].lower()), 'Standard')
        try:
            loaded = []
            for dae in daes:
                parts = parts_of(dae)
                # The file holds all four wheels (TireK_RF/RB/LF/LB); keep one.
                loaded.append([p for p in parts if '_LF_' in p[0]] or parts[:1])
            variants = [(names[0], loaded[0])]
            if len(names) > 1:
                if len(loaded) > 1:
                    variants.append((names[1], loaded[1]))
                else:
                    # Same model, other albedo (Tire_Std_Alb -> Tire_Zst_Alb).
                    used = {p[1] for p in loaded[0] if p[1] is not None and p[1].name.lower().startswith('tire_')}
                    others = sorted(p for p in folder.rglob('*') if p.name.lower().startswith('tire_')
                                    and '_alb' in p.name.lower() and p not in used)
                    if used and others:
                        variants.append((names[1], restyle(loaded[0], next(iter(used)), others[0])))
            for name, parts in variants:
                t_low, t_high = bounds(parts)
                parts = transform(parts, common, -(t_low + t_high) / 2 * common)
                t_low, t_high = bounds(parts)
                size = t_high - t_low
                radius = float(max(size[1], size[2]) / 2)  # axle along X
                file = f'parts/tyres/{slug(name)}.glb'
                write_glb(parts, args.mod / file, 1.0)
                render_preview(parts, args.mod / file.replace('.glb', '.png'), yaw=-1.1, pitch=0.2)
                listing['tyres'].append({'name': name, 'file': file, 'radius': radius, 'grip': GRIP[family],
                                         'scale': 0.85 if name in SMALLER else 1.0,
                                         'preview': file.replace('.glb', '.png')})
                print('tyre', name, f'radius {radius:.3f} axle width {size[0]:.3f}')
        except Exception as error:
            print('SKIP tyre', folder.name, error)

    listing['original_preview'] = 'parts/bodies/original.png'
    (out / 'parts.json').write_text(json.dumps(listing, indent=2), encoding='utf-8')
    print(f"{len(listing['bodies'])} bodies, {len(listing['tyres'])} tyres, {len(listing['drivers'])} drivers")


if __name__ == '__main__':
    main()
