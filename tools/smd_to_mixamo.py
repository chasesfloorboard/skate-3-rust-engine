"""Rebuild a Super Mario Odyssey character (Valve SMD rip: the body .SMD plus
its hand/eyebrow part .SMDs) as a Mixamo-named skinned .glb, for
tools/mk8_character.py (keep-proportions import).

The Odyssey rig (Hip, LegL1/LegL2/FootL/ToeL, Spine1, Spine2, Head,
ShoulderL/ArmL1/ArmL2/HandL with *Sub twist joints) maps onto Mixamo's
humanoid names; Mixamo's Spine2 and Neck are added between the chest and
the head, unweighted. Twist joints fold into their parent and the jaw into
the head. The cap has no joint in the rip: its vertices get a Cap joint on
the head, which the game's spring bones (jiggle.rs) lift off and drop in
bails (Odyssey characters have real hair under their caps). Other extra
joints (bags, tails) stay as themselves and become spring bones too.

Usage:
  python tools/smd_to_mixamo.py BODY.SMD OUT.glb [--parts A.SMD B.SMD ...]
"""
import argparse
import re
from pathlib import Path

import numpy as np
import pygltflib as gl

MIXAMO = {
    'Hip': 'Hips', 'Spine1': 'Spine', 'Spine2': 'Spine1', 'Head': 'Head',
    'ShoulderL': 'LeftShoulder', 'ArmL1': 'LeftArm', 'ArmL2': 'LeftForeArm', 'HandL': 'LeftHand',
    'ShoulderR': 'RightShoulder', 'ArmR1': 'RightArm', 'ArmR2': 'RightForeArm', 'HandR': 'RightHand',
    'LegL1': 'LeftUpLeg', 'LegL2': 'LeftLeg', 'FootL': 'LeftFoot', 'ToeL': 'LeftToeBase',
    'LegR1': 'RightUpLeg', 'LegR2': 'RightLeg', 'FootR': 'RightFoot', 'ToeR': 'RightToeBase',
}
# Joints folded into another (weights move there; no joint of their own).
FOLD_TO_HEAD = ('Joe', 'Jaw', 'Mouth', 'Eye', 'Brow', 'Nose')
ROOTS = ('nw4f_root', 'AllRoot', 'JointRoot')


def euler(rx, ry, rz):
    """SMD rotation (radians, applied X then Y then Z)."""
    cx, sx, cy, sy, cz, sz = np.cos(rx), np.sin(rx), np.cos(ry), np.sin(ry), np.cos(rz), np.sin(rz)
    x = np.array([[1, 0, 0], [0, cx, -sx], [0, sx, cx]])
    y = np.array([[cy, 0, sy], [0, 1, 0], [-sy, 0, cy]])
    z = np.array([[cz, -sz, 0], [sz, cz, 0], [0, 0, 1]])
    return z @ y @ x


def parse(path):
    """(nodes {id: (name, parent)}, bind locals {id: 4x4}, triangles
    [(material, [(pos, normal, uv, [(bone, weight)]) x3])])."""
    nodes, locals_, triangles = {}, {}, []
    section, time = None, None
    lines = Path(path).read_text(errors='ignore').splitlines()
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        i += 1
        if line in ('nodes', 'skeleton', 'triangles'):
            section = line
            continue
        if line == 'end':
            section = None
            continue
        if section == 'nodes':
            m = re.match(r'(\d+)\s+"([^"]*)"\s+(-?\d+)', line)
            if m:
                nodes[int(m.group(1))] = (m.group(2), int(m.group(3)))
        elif section == 'skeleton':
            if line.startswith('time'):
                time = int(line.split()[1])
            elif time == 0 and line:
                v = line.split()
                m = np.eye(4)
                m[:3, :3] = euler(*map(float, v[4:7]))
                m[:3, 3] = list(map(float, v[1:4]))
                locals_[int(v[0])] = m
        elif section == 'triangles' and line:
            material = line
            corners = []
            for _ in range(3):
                v = lines[i].split()
                i += 1
                pos, nrm, uv = np.array(v[1:4], float), np.array(v[4:7], float), np.array(v[7:9], float)
                links = []
                if len(v) > 9:
                    count = int(v[9])
                    links = [(int(v[10 + 2 * k]), float(v[11 + 2 * k])) for k in range(count)]
                if not links:
                    links = [(int(v[0]), 1.0)]
                corners.append((pos, nrm, uv, links))
            triangles.append((material, corners))
    return nodes, locals_, triangles


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('model', type=Path)
    parser.add_argument('out', type=Path)
    parser.add_argument('--parts', type=Path, nargs='*', default=[])
    parser.add_argument('--scale', type=float, default=0.01, help='rip units to metres (cm)')
    args = parser.parse_args()
    nodes, locals_, triangles = parse(args.model)
    for part in args.parts:
        _, _, extra = parse(part)
        triangles += extra
    by_name = {name: i for i, (name, _) in nodes.items()}
    worlds = {}

    def world(i):
        if i not in worlds:
            name, parent = nodes[i]
            local = locals_.get(i, np.eye(4))
            worlds[i] = (world(parent) @ local) if parent >= 0 else local
        return worlds[i]
    for i in nodes:
        world(i)
    # Vertices are stored in model space; joint worlds include the root's
    # Z-up turn. Bring both into Y-up metres: find up from hips to head.
    hip, head = worlds[by_name['Hip']][:3, 3], worlds[by_name['Head']][:3, 3]
    up_axis = int(np.argmax(np.abs(head - hip)))
    if up_axis == 2:
        convert = np.array([[1, 0, 0, 0], [0, 0, 1, 0], [0, -1, 0, 0], [0, 0, 0, 1.]])
    else:
        convert = np.eye(4)
    scale = np.diag([args.scale] * 3 + [1.0])
    frame = scale @ convert

    # Skeleton: body joints (Mixamo names) and extras; roots, twist and face
    # helpers fold away.
    def target(i):
        name, parent = nodes[i]
        if name in MIXAMO:
            return name
        if name in ROOTS or parent < 0:
            return None
        if name.endswith('Sub') or any(name.startswith(k) for k in FOLD_TO_HEAD):
            return target(parent) if not any(name.startswith(k) for k in FOLD_TO_HEAD) else 'Head'
        return name  # an extra (bag, tail...) kept for spring bones
    keep = {}
    for i, (name, parent) in nodes.items():
        if name == args.model.stem or i not in locals_:
            continue
        t = target(i)
        if t is not None:
            keep.setdefault(t, i)
    joint_world = {t: frame @ worlds[i] for t, i in keep.items()}
    # Pure positions (stock orientation is fitted later): no rotation.
    for t, m in joint_world.items():
        joint_world[t] = np.eye(4)
        joint_world[t][:3, 3] = m[:3, 3]
    parents = {}
    for t, i in keep.items():
        parent = nodes[i][1]
        while parent >= 0 and (target(parent) is None or target(parent) == t):
            parent = nodes[parent][1]
        parents[t] = target(parent) if parent >= 0 else None
        # Odyssey hangs the spine beside the hips under the rig root; Mixamo
        # (and the importer) want everything under the hips.
        if parents[t] is None and t != 'Hip':
            parents[t] = 'Hip'
    # Mixamo's upper chest and neck between the chest (Spine2) and the head.
    chest, top = joint_world['Spine2'][:3, 3], joint_world['Head'][:3, 3]
    for n, (name, share) in enumerate((('SyntheticSpine2', 1 / 3), ('Neck', 2 / 3))):
        m = np.eye(4)
        m[:3, 3] = chest + (top - chest) * share
        joint_world[name] = m
    parents['SyntheticSpine2'] = 'Spine2'
    parents['Neck'] = 'SyntheticSpine2'
    parents['Head'] = 'Neck'
    for t in list(parents):
        if parents[t] == 'Spine2' and t not in ('SyntheticSpine2',) and t.startswith('Shoulder'):
            parents[t] = 'SyntheticSpine2'
    # The cap: its own joint on the head, at the cap's centre.
    cap_material = next((m for m, _ in triangles if 'cap' in m.lower()), None)
    if cap_material:
        points = np.array([c[0] for m, cs in triangles if m == cap_material for c in cs])
        centre = (frame @ np.c_[points, np.ones(len(points))].T).T[:, :3].mean(0)
        m = np.eye(4)
        m[:3, 3] = centre
        joint_world['Cap'] = m
        parents['Cap'] = 'Head'
    order = []

    def add(t):
        if t in order or t not in joint_world:
            return
        if parents.get(t):
            add(parents[t])
        order.append(t)
    add('Hip')
    for t in joint_world:
        add(t)
    index = {t: n for n, t in enumerate(order)}
    rename = {**MIXAMO, 'SyntheticSpine2': 'Spine2', 'Neck': 'Neck'}

    gltf = gl.GLTF2(scene=0, scenes=[gl.Scene(nodes=[])], asset=gl.Asset(generator='smd_to_mixamo'))
    blob = bytearray()

    def view(data, target=None):
        while len(blob) % 4:
            blob.append(0)
        gltf.bufferViews.append(gl.BufferView(buffer=0, byteOffset=len(blob), byteLength=len(data), target=target))
        blob.extend(data)
        return len(gltf.bufferViews) - 1

    def accessor(array, kind, component=gl.FLOAT, target=gl.ARRAY_BUFFER, bounds=False):
        array = np.ascontiguousarray(array)
        gltf.accessors.append(gl.Accessor(bufferView=view(array.tobytes(), target), componentType=component,
                                          count=len(array), type=kind,
                                          min=array.min(0).tolist() if bounds else None,
                                          max=array.max(0).tolist() if bounds else None))
        return len(gltf.accessors) - 1
    for t in order:
        parent = parents.get(t)
        local = np.linalg.inv(joint_world[parent]) @ joint_world[t] if parent in joint_world else joint_world[t]
        gltf.nodes.append(gl.Node(name='mixamorig:' + rename.get(t, t), matrix=local.T.reshape(-1).tolist()))
    for t in order:
        parent = parents.get(t)
        if parent in index:
            node = gltf.nodes[index[parent]]
            node.children = (node.children or []) + [index[t]]
    roots = [index[t] for t in order if parents.get(t) not in index]
    gltf.scenes[0].nodes.extend(roots)
    inverse = np.stack([np.linalg.inv(joint_world[t]).T for t in order]).astype(np.float32)
    gltf.skins.append(gl.Skin(joints=list(range(len(order))), inverseBindMatrices=accessor(inverse, gl.MAT4, target=None),
                              skeleton=roots[0]))

    images = args.model.parent / 'images'
    textures = {}
    undercap_texture = {}
    by_material = {}
    for material, corners in triangles:
        by_material.setdefault(material, []).extend(corners)
    # Hair inside the cap: its own "UnderCap" material, which the game hides
    # while the cap is on (jiggle.rs) and shows once it comes off. A hair
    # triangle is under the cap when the cap's surface lies further out from
    # the head's centre in the same direction.
    if cap_material:
        to_frame = lambda pts: (frame @ np.c_[pts, np.ones(len(pts))].T).T[:, :3]
        cap_points = to_frame(np.array([c[0] for c in by_material[cap_material]]))
        centre = joint_world['Head'][:3, 3].copy()
        centre[1] = (cap_points[:, 1].min() + centre[1]) / 2
        cap_dirs = cap_points - centre
        cap_reach = np.linalg.norm(cap_dirs, axis=1)
        cap_dirs /= np.maximum(cap_reach[:, None], 1e-9)
        for material in [m for m in by_material if 'hair' in m.lower() and m != cap_material]:
            corners = by_material[material]
            keep, under = [], []
            for t in range(0, len(corners), 3):
                tri = corners[t:t + 3]
                c = to_frame(np.array([v[0] for v in tri])).mean(0)
                d = c - centre
                r = np.linalg.norm(d)
                near = cap_dirs @ (d / max(r, 1e-9)) > np.cos(np.radians(10))
                (under if near.any() and cap_reach[near].max() >= r - 0.004 / args.scale * 0.01 else keep).extend(tri)
            by_material[material] = keep
            if under:
                by_material[Path(material).stem + '_UnderCap.png'] = under
                # Same texture as the hair it came from.
                undercap_texture[Path(material).stem + '_UnderCap.png'] = material
    for material, corners in [(m, c) for m, c in by_material.items() if c]:
        positions = np.array([c[0] for c in corners])
        positions = (frame @ np.c_[positions, np.ones(len(positions))].T).T[:, :3].astype(np.float32)
        normals = np.array([c[1] for c in corners]) @ convert[:3, :3].T
        normals = (normals / np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-9)).astype(np.float32)
        uvs = np.array([[c[2][0], 1.0 - c[2][1]] for c in corners], np.float32)
        joints = np.zeros((len(corners), 4), np.uint16)
        weights = np.zeros((len(corners), 4), np.float32)
        for v, (_, _, _, links) in enumerate(corners):
            merged = {}
            for bone, w in links:
                t = 'Cap' if material == cap_material else (target(bone) if bone in nodes else None)
                while t is not None and t not in index:
                    t = parents.get(t)
                merged[index.get(t, index['Hip'])] = merged.get(index.get(t, index['Hip']), 0.0) + w
            pairs = sorted(merged.items(), key=lambda p: -p[1])[:4]
            total = sum(w for _, w in pairs) or 1.0
            for k, (j, w) in enumerate(pairs):
                joints[v, k], weights[v, k] = j, w / total
        attributes = gl.Attributes(POSITION=accessor(positions, gl.VEC3, bounds=True), NORMAL=accessor(normals, gl.VEC3),
                                   TEXCOORD_0=accessor(uvs, gl.VEC2),
                                   JOINTS_0=accessor(joints, gl.VEC4, component=gl.UNSIGNED_SHORT),
                                   WEIGHTS_0=accessor(weights, gl.VEC4))
        mat = gl.Material(name=Path(material).stem, doubleSided=True,
                          pbrMetallicRoughness=gl.PbrMetallicRoughness(metallicFactor=0.0, roughnessFactor=0.7))
        source = undercap_texture.get(material, material)
        texture = next((p for p in images.glob('*') if p.name.lower() == source.lower()), None)
        if texture is not None:
            if texture not in textures:
                gltf.images.append(gl.Image(bufferView=view(texture.read_bytes()), mimeType='image/png'))
                gltf.textures.append(gl.Texture(source=len(gltf.images) - 1))
                textures[texture] = len(gltf.textures) - 1
            mat.pbrMetallicRoughness.baseColorTexture = gl.TextureInfo(index=textures[texture])
        gltf.materials.append(mat)
        gltf.meshes.append(gl.Mesh(name=Path(material).stem, primitives=[gl.Primitive(attributes=attributes, material=len(gltf.materials) - 1)]))
        gltf.nodes.append(gl.Node(name=Path(material).stem, mesh=len(gltf.meshes) - 1, skin=0))
        gltf.scenes[0].nodes.append(len(gltf.nodes) - 1)
    gltf.buffers.append(gl.Buffer(byteLength=len(blob)))
    gltf.set_binary_blob(bytes(blob))
    gltf.save_binary(str(args.out))
    height = joint_world['Head'][1, 3] - min(m[1, 3] for m in joint_world.values())
    print(f'{args.out}: {len(order)} joints ({", ".join(t for t in order if t not in MIXAMO)}), '
          f'{len(gltf.meshes)} meshes, head {height:.2f} m above the lowest joint')


if __name__ == '__main__':
    main()
