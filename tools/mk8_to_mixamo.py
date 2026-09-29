"""Rebuild a Mario Kart 8 racer (Collada rip) as a Mixamo-named skinned .glb,
so the game's character importer (tools/mixamo_to_skate) can fit it to the
skater rig and it skates, animates and bails like any imported character.

The MK8 racer rig (Skl_Root, Hip, LegL/KneeL/FootL, Spine1, Spine2, Head,
ShoulderL/ArmL/ElbowL/HandL, fingers, face helpers) maps onto Mixamo's
humanoid names; the Mixamo spine has two more joints (Spine2, Neck) than the
MK8 one, so those are added between the chest and the head, unweighted. The
pupils hang off the model root in the rip and are re-parented to the head.
The model is written in its bind (T) pose, Y-up, in metres.

Usage:
  python tools/mk8_to_mixamo.py RACER.dae OUT.glb [--scale 0.075]
"""
import argparse
import os
import re
import sys
from pathlib import Path

import collada
import numpy as np
import pygltflib as gl

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mk8_to_glb import Rig, Y_UP, albedo_for, canon  # noqa: E402

RENAME = {
    'Skl_Root_1': 'Hips', 'Head_1': 'Head',
    'ShoulderL_1': 'LeftShoulder', 'ArmL_1': 'LeftArm', 'ElbowL_1': 'LeftForeArm', 'HandL_1': 'LeftHand',
    'ShoulderR_1': 'RightShoulder', 'ArmR_1': 'RightArm', 'ElbowR_1': 'RightForeArm', 'HandR_1': 'RightHand',
    'LegL_1': 'LeftUpLeg', 'KneeL_1': 'LeftLeg', 'FootL_1': 'LeftFoot',
    'LegR_1': 'RightUpLeg', 'KneeR_1': 'RightLeg', 'FootR_1': 'RightFoot',
    'Finger1L_1': 'LeftHandIndex1', 'Finger2L_1': 'LeftHandIndex2',
    'Thumb1L_1': 'LeftHandThumb1', 'Thumb2L_1': 'LeftHandThumb2',
    'Finger1R_1': 'RightHandIndex1', 'Finger2R_1': 'RightHandIndex2',
    'Thumb1R_1': 'RightHandThumb1', 'Thumb2R_1': 'RightHandThumb2',
}
# Mixamo's spine below the head, filled from the racer's own spine joints.
SPINE = ['Spine', 'Spine1', 'Spine2', 'Neck']


def scalp(gltf, cap, body, accessor, view, head, hair=None):
    """A bald crown with a comb-over under a racer's cap: the rips model no
    head under the cap, so the head was open when the cap came off. An
    ellipsoid dome over the head mesh's open top (its highest ring under the
    cap), reaching up inside the crown, skinned to the head."""
    import io
    from PIL import Image, ImageDraw
    top = cap[:, 1].max()
    low, high = cap.min(0), cap.max(0)
    under = body[(body[:, 0] > low[0]) & (body[:, 0] < high[0]) & (body[:, 2] > low[2]) & (body[:, 2] < high[2])]
    rim_y = under[:, 1].max()
    # Width from the cap's crown above that opening (not its brim).
    crown = cap[cap[:, 1] > rim_y]
    if len(crown) < 10:
        crown = under[under[:, 1] > rim_y - 0.04]
    centre = (crown.min(0) + crown.max(0)) / 2
    rx, rz = (crown.max(0) - crown.min(0))[[0, 2]] / 2 * 0.9
    base = rim_y - 0.03
    height = 0.8 * (top - base)
    rings, segments = 10, 32
    positions, normals, uvs = [], [], []
    for i in range(rings + 1):
        phi = (np.pi / 2) * i / rings
        for k in range(segments + 1):
            theta = 2 * np.pi * k / segments
            x, z = np.cos(theta) * np.cos(phi), np.sin(theta) * np.cos(phi)
            y = np.sin(phi)
            positions.append([centre[0] + rx * x, base + height * y, centre[2] + rz * z])
            n = np.array([x / rx, y / height, z / rz])
            normals.append(n / np.linalg.norm(n))
            # Planar map from above: u across the head, v front to back.
            uvs.append([0.5 + 0.5 * x, 0.5 + 0.5 * z])
    triangles = []
    for i in range(rings):
        for k in range(segments):
            a, b = i * (segments + 1) + k, (i + 1) * (segments + 1) + k
            triangles += [[a, b, a + 1], [a + 1, b, b + 1]]
    corners = np.array(triangles).reshape(-1)
    positions = np.array(positions, np.float32)[corners]
    normals = np.array(normals, np.float32)[corners]
    uvs = np.array(uvs, np.float32)[corners]
    if hair is None:
        # Bald skin with a few dark strands combed across the crown.
        image = Image.new('RGB', (128, 128), (250, 196, 150))
        draw = ImageDraw.Draw(image)
        for strand in range(5):
            v = 34 + strand * 13
            points = [(u, v + 4 * np.sin(u / 18 + strand)) for u in range(10, 120, 4)]
            draw.line(points, fill=(70, 38, 18), width=4)
    else:
        # A full head of hair combed back, in the racer's hair colour.
        image = Image.new('RGB', (128, 128), hair)
        draw = ImageDraw.Draw(image)
        dark = tuple(int(c * 0.6) for c in hair)
        light = tuple(min(255, int(c * 1.35) + 10) for c in hair)
        for strand in range(0, 128, 6):
            points = [(strand + 3 * np.sin(v / 14 + strand), v) for v in range(0, 128, 4)]
            draw.line(points, fill=dark if strand % 12 else light, width=2)
    data = io.BytesIO()
    image.save(data, 'PNG')
    gltf.images.append(gl.Image(bufferView=view(data.getvalue()), mimeType='image/png'))
    gltf.textures.append(gl.Texture(source=len(gltf.images) - 1))
    material = gl.Material(name='Scalp', doubleSided=True, pbrMetallicRoughness=gl.PbrMetallicRoughness(
        metallicFactor=0.0, roughnessFactor=0.7, baseColorTexture=gl.TextureInfo(index=len(gltf.textures) - 1)))
    # Metal racers (Metal Mario): the scalp is metal too, reflecting the
    # body's sphere map over a dark base like the body's own albedo.
    metal = next((m for m in gltf.materials if m.emissiveTexture is not None
                  and m.pbrMetallicRoughness and m.pbrMetallicRoughness.metallicFactor == 1.0), None)
    if metal is not None:
        data = io.BytesIO()
        Image.new('RGB', (8, 8), (8, 8, 8)).save(data, 'PNG')
        gltf.images.append(gl.Image(bufferView=view(data.getvalue()), mimeType='image/png'))
        gltf.textures.append(gl.Texture(source=len(gltf.images) - 1))
        material.pbrMetallicRoughness = gl.PbrMetallicRoughness(metallicFactor=1.0, roughnessFactor=0.2,
                                                                baseColorTexture=gl.TextureInfo(index=len(gltf.textures) - 1))
        material.emissiveTexture = gl.TextureInfo(index=metal.emissiveTexture.index)
        material.emissiveFactor = [0.0, 0.0, 0.0]
    gltf.materials.append(material)
    count = len(positions)
    joints = np.zeros((count, 4), np.uint16)
    joints[:, 0] = head
    weights = np.zeros((count, 4), np.float32)
    weights[:, 0] = 1.0
    attributes = gl.Attributes(POSITION=accessor(positions, gl.VEC3, bounds=True), NORMAL=accessor(normals, gl.VEC3),
                               TEXCOORD_0=accessor(uvs, gl.VEC2),
                               JOINTS_0=accessor(joints, gl.VEC4, component=gl.UNSIGNED_SHORT),
                               WEIGHTS_0=accessor(weights, gl.VEC4))
    gltf.meshes.append(gl.Mesh(name='Scalp', primitives=[gl.Primitive(attributes=attributes, material=len(gltf.materials) - 1)]))
    gltf.nodes.append(gl.Node(name='Scalp', mesh=len(gltf.meshes) - 1, skin=0))
    gltf.scenes[0].nodes.append(len(gltf.nodes) - 1)


def axis_fix(primitives, bind_shape, dominant, joint_position):
    """The axis-aligned rotation (of 24) that best seats bind-shape vertices on
    the joints weighting them most."""
    import itertools
    vertices = np.concatenate([p.vertex for p in primitives if hasattr(p, 'vertex_index')])[:len(dominant)]
    vertices = (np.c_[vertices, np.ones(len(vertices))] @ bind_shape.T)[:, :3]
    step = max(1, len(vertices) // 600)
    samples = [(v, joint_position(n)) for v, n in zip(vertices[::step], dominant[::step])]
    samples = [(v, j) for v, j in samples if j is not None]
    best, best_cost = np.eye(4), None
    for perm in itertools.permutations(range(3)):
        for signs in itertools.product((1, -1), repeat=3):
            r = np.zeros((3, 3))
            for row, (col, sign) in enumerate(zip(perm, signs)):
                r[row, col] = sign
            if np.linalg.det(r) < 0:
                continue
            cost = sum(np.linalg.norm(r @ v - j) for v, j in samples)
            if best_cost is None or cost < best_cost - 1e-6 * abs(cost):
                best_cost = cost
                best = np.eye(4)
                best[:3, :3] = r
    return best


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('model', type=Path)
    parser.add_argument('out', type=Path)
    parser.add_argument('--scale', type=float, default=0.075)
    parser.add_argument('--hair', default=None, help='RRGGBB hair colour under a cap (default: bald with a comb-over)')
    args = parser.parse_args()
    folder = args.model.parent
    document = collada.Collada(str(args.model), ignore=[collada.common.DaeUnsupportedError, collada.common.DaeBrokenRefError])
    rig = Rig(document, [])
    # Rips number joints per export (Head_1, Head_2...); use one suffix.

    # The racer's skeleton is the Skl_Root subtree. Weighted joints outside it
    # (the model root, stray pupils/eyes) join the head (eyes) or the hips.
    parents = {name: parent for name, (parent, _) in rig.nodes.items()}
    def inside(name):
        while name:
            if name == 'Skl_Root_1':
                return True
            name = parents.get(name)
        return False
    used = set()
    for controller in document.controllers:
        used.update(canon(str(n)) for n in controller.weight_joints.data.reshape(-1))
    fold = {}
    for name in sorted(used):
        if not inside(name):
            eye = any(k in name.lower() for k in ('pupil', 'eye'))
            fold[name] = 'Head_1' if eye and 'Head_1' in rig.nodes else 'Skl_Root_1'
    used = {name for name in used if name not in fold}
    parents['Skl_Root_1'] = None
    joints = []

    def add(name):
        if name in joints or name not in rig.nodes:
            return
        parent = parents.get(name)
        if parent and parent in rig.nodes and inside(parent):
            add(parent)
        joints.append(name)
    for name in sorted(used):
        add(name)
    add('Head_1')
    # Bind worlds in metres, Y-up, pure rotations (MK8 bind frames carry scale).
    # Rips disagree on spaces (Rosalina's mesh is Y-up, her skeleton Z-up):
    # the skin's inverse binds tie joints to the mesh, so carry the skeleton
    # into mesh space with them, then stand the result up (hips to head = +Y).
    to_mesh = np.eye(4)
    for controller in document.controllers:
        shared = [n for n in controller.joint_matrices if canon(str(n)) in rig.nodes]
        if shared:
            n = shared[0]
            to_mesh = np.linalg.inv(np.array(controller.joint_matrices[n])) @ np.linalg.inv(rig.world(canon(str(n))))
            break
    def mesh_world(name):
        return to_mesh @ rig.world(name)
    top = next((n for n in ('Hip_1', 'Skl_Root_1') if n in rig.nodes), None)
    up = mesh_world('Head_1')[:3, 3] - mesh_world(top)[:3, 3] if top and 'Head_1' in rig.nodes else np.array([0., 0., 1.])
    up /= np.linalg.norm(up)
    v, c = np.cross(up, [0., 1., 0.]), float(np.dot(up, [0., 1., 0.]))
    k = np.array([[0, -v[2], v[1]], [v[2], 0, -v[0]], [-v[1], v[0], 0]])
    stand = np.eye(4)
    stand[:3, :3] = np.eye(3) + k + k @ k / (1 + c) if c > -0.999 else np.diag([1., -1., -1.])

    def bind(name):
        m = stand @ mesh_world(name)
        m = m.copy()
        m[:3, 3] *= args.scale
        m[:3, :3] /= np.linalg.norm(m[:3, :3], axis=0)
        return m
    worlds = {name: bind(name) for name in joints}
    # The hips pivot at the pelvis joint (some racers root Skl_Root at the feet).
    if 'Hip_1' in worlds:
        worlds['Skl_Root_1'][:3, 3] = worlds['Hip_1'][:3, 3]
    # Spine: the racer's own joints between the root and the head, then
    # synthetic ones up to the four Mixamo expects, spread toward the head.
    chain = []
    name = parents.get('Head_1')
    while name and name != 'Skl_Root_1':
        chain.insert(0, name)
        name = parents.get(name)
    chain = chain[:len(SPINE)]
    rename = dict(RENAME)
    for joint, mixamo in zip(chain, SPINE):
        rename[joint] = mixamo
    last = chain[-1] if chain else 'Skl_Root_1'
    missing = SPINE[len(chain):]
    top, head = worlds[last], worlds['Head_1']
    previous = last
    for n, mixamo in enumerate(missing, 1):
        m = top.copy()
        m[:3, 3] = top[:3, 3] + (head[:3, 3] - top[:3, 3]) * n / (len(missing) + 1)
        worlds[mixamo] = m
        parents[mixamo] = previous
        rename[mixamo] = mixamo
        previous = mixamo
    parents['Head_1'] = previous
    order = []
    for name in joints:
        if name == 'Head_1':
            order += missing
        order.append(name)
    for name in list(fold):
        fold[name] = fold[name] if fold[name] in worlds else 'Skl_Root_1'
    index = {name: i for i, name in enumerate(order)}

    gltf = gl.GLTF2(scene=0, scenes=[gl.Scene(nodes=[])], asset=gl.Asset(generator='mk8_to_mixamo'))
    blob = bytearray()

    def view(data: bytes, target=None):
        while len(blob) % 4:
            blob.append(0)
        gltf.bufferViews.append(gl.BufferView(buffer=0, byteOffset=len(blob), byteLength=len(data), target=target))
        blob.extend(data)
        return len(gltf.bufferViews) - 1

    def accessor(array, kind, component=gl.FLOAT, target=gl.ARRAY_BUFFER, bounds=False):
        gltf.accessors.append(gl.Accessor(bufferView=view(array.tobytes(), target), componentType=component,
                                          count=len(array), type=kind,
                                          min=array.min(0).tolist() if bounds else None,
                                          max=array.max(0).tolist() if bounds else None))
        return len(gltf.accessors) - 1

    # Skeleton nodes (first), with local transforms from the bind worlds.
    for name in order:
        parent = parents.get(name)
        local = np.linalg.inv(worlds[parent]) @ worlds[name] if parent in worlds else worlds[name]
        gltf.nodes.append(gl.Node(name='mixamorig:' + rename.get(name, name).replace(':', '_'),
                                  matrix=local.T.reshape(-1).tolist()))
    for name in order:
        parent = parents.get(name)
        if parent in index:
            node = gltf.nodes[index[parent]]
            node.children = (node.children or []) + [index[name]]
    roots = [index[n] for n in order if parents.get(n) not in index]
    gltf.scenes[0].nodes.extend(roots)
    inverse = np.stack([np.linalg.inv(worlds[n]).T for n in order]).astype(np.float32)
    gltf.skins.append(gl.Skin(joints=list(range(len(order))), inverseBindMatrices=accessor(inverse, gl.MAT4, target=None),
                              skeleton=roots[0]))

    images = {i.id: folder / i.path for i in document.images}
    textures = {}
    # Eyes and pupils map UVs past 0..1 (MK8 clamps them); repeating showed
    # rows of extra eyes.
    gltf.samplers = [gl.Sampler(wrapS=gl.CLAMP_TO_EDGE, wrapT=gl.CLAMP_TO_EDGE)]
    controllers = [c for c in document.controllers if not c.geometry.id.endswith(('_001-mesh', '_002-mesh'))]
    # The eye texture already paints the pupils; the separate pupil planes
    # (moved by MK8's eye shader) sat on it z-fighting or showed a second pair.
    if os.environ.get('MK8_KEEP_PUPILS') is None:
        controllers = [c for c in controllers if 'pupil' not in c.geometry.id.lower()]

    def controller_fix(controller):
        # Some rips store vertices in another axis convention than their
        # skeleton (Rosalina: Y-up mesh, Z-up joints). Pick the axis-aligned
        # rotation that puts vertices nearest the joint weighting them most.
        names = [canon(str(n)) for n in controller.weight_joints.data.reshape(-1)]
        weights_source = controller.weights.data.reshape(-1)
        dominant = np.array([names[max(pairs, key=lambda p: weights_source[p[1]])[0]] if len(pairs) else 'Skl_Root_1'
                             for pairs in controller.index])
        return axis_fix(controller.geometry.primitives, np.array(controller.bind_shape_matrix).reshape(4, 4), dominant,
                        lambda n: mesh_world(fold.get(n, n))[:3, 3] if fold.get(n, n) in rig.nodes else None)

    # One rotation for the whole model, from its biggest mesh: small meshes on
    # the head joint (eyes, pupils, noses) fit several rotations about equally
    # and came out turned into or behind the head.
    fix = controller_fix(max(controllers, key=lambda c: len(c.index)))
    cap_joint = next((index[n] for n in index if re.match(r'Cap(_\d+)?$', n)), None)
    capped, uncapped = [], []
    for controller in controllers:
        geometry = controller.geometry
        names = [canon(str(n)) for n in controller.weight_joints.data.reshape(-1)]
        weights_source = controller.weights.data.reshape(-1)
        bind_shape = np.array(controller.bind_shape_matrix).reshape(4, 4)
        # Per source vertex: up to four (joint, weight) pairs.
        vj = np.zeros((len(controller.index), 4), np.uint16)
        vw = np.zeros((len(controller.index), 4), np.float32)
        for v, pairs in enumerate(controller.index):
            merged = {}
            for j, w in pairs:
                joint = index.get(fold.get(names[j], names[j]), index['Skl_Root_1'])
                merged[joint] = merged.get(joint, 0.0) + weights_source[w]
            pairs = sorted(merged.items(), key=lambda p: -p[1])[:4]
            total = sum(w for _, w in pairs) or 1.0
            for k, (j, w) in enumerate(pairs):
                vj[v, k], vw[v, k] = j, w / total
        for primitive in geometry.primitives:
            corners = primitive.vertex_index.reshape(-1)
            positions = np.c_[primitive.vertex, np.ones(len(primitive.vertex))] @ bind_shape.T @ fix.T @ stand.T
            positions = (positions[:, :3] * args.scale)[corners].astype(np.float32)
            normals = primitive.normal[primitive.normal_index.reshape(-1)] @ bind_shape[:3, :3].T @ fix[:3, :3].T @ stand[:3, :3].T
            normals = (normals / np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-9)).astype(np.float32)
            uvs = primitive.texcoordset[0][primitive.texcoord_indexset[0].reshape(-1)]
            uvs = np.c_[uvs[:, 0], 1.0 - uvs[:, 1]].astype(np.float32)
            if cap_joint is not None:
                on_cap = (vj[corners][:, 0] == cap_joint) & (vw[corners][:, 0] > 0.5)
                capped.append(positions[on_cap])
                uncapped.append(positions[~on_cap])
            attributes = gl.Attributes(POSITION=accessor(positions, gl.VEC3, bounds=True), NORMAL=accessor(normals, gl.VEC3),
                                       TEXCOORD_0=accessor(uvs, gl.VEC2),
                                       JOINTS_0=accessor(vj[corners], gl.VEC4, component=gl.UNSIGNED_SHORT),
                                       WEIGHTS_0=accessor(vw[corners], gl.VEC4))
            material = gl.Material(name=primitive.material, doubleSided=True,
                                   pbrMetallicRoughness=gl.PbrMetallicRoughness(metallicFactor=0.0, roughnessFactor=0.7))
            effect = next((m.effect for m in document.materials if m.id == primitive.material), None)
            if effect is not None and isinstance(effect.diffuse, collada.material.Map):
                texture = albedo_for(images[effect.diffuse.sampler.surface.image.id], folder, 'A')
                # Eyes: the rip lists every expression; frame 0 is the open eye.
                if not texture.is_file():
                    texture = next(iter(sorted(folder.glob(texture.stem.split('.')[0] + '.0.png'))), texture)
                if texture.is_file():
                    clamp = any(k in (primitive.material or '').lower() for k in ('eye', 'pupil'))
                    if (texture, clamp) not in textures:
                        gltf.images.append(gl.Image(bufferView=view(texture.read_bytes()), mimeType='image/png'))
                        gltf.textures.append(gl.Texture(source=len(gltf.images) - 1, sampler=0 if clamp else None))
                        textures[(texture, clamp)] = len(gltf.textures) - 1
                    material.pbrMetallicRoughness.baseColorTexture = gl.TextureInfo(index=textures[(texture, clamp)])
                    # Metal racers (Metal Mario, Pink Gold Peach) reflect a
                    # sphere map, *_mlt beside their albedo. It rides in the
                    # emissive slot at zero strength; the game reads metallic
                    # materials' emissive texture as their reflection matcap.
                    stem = texture.name.split('_alb')[0]
                    matcap = next((m for m in sorted(texture.parent.glob('*_mlt.png'))
                                   if stem.startswith(m.name[:-len('_mlt.png')])), texture.with_name(stem + '_mlt.png'))
                    if '_alb' in texture.name and matcap.is_file():
                        if matcap not in textures:
                            gltf.images.append(gl.Image(bufferView=view(matcap.read_bytes()), mimeType='image/png'))
                            gltf.textures.append(gl.Texture(source=len(gltf.images) - 1))
                            textures[matcap] = len(gltf.textures) - 1
                        material.pbrMetallicRoughness.metallicFactor = 1.0
                        material.pbrMetallicRoughness.roughnessFactor = 0.2
                        material.emissiveTexture = gl.TextureInfo(index=textures[matcap])
                        material.emissiveFactor = [0.0, 0.0, 0.0]
            gltf.materials.append(material)
            gltf.meshes.append(gl.Mesh(name=geometry.id, primitives=[gl.Primitive(attributes=attributes, material=len(gltf.materials) - 1)]))
            gltf.nodes.append(gl.Node(name=geometry.id, mesh=len(gltf.meshes) - 1, skin=0))
            gltf.scenes[0].nodes.append(len(gltf.nodes) - 1)
    if cap_joint is not None and capped and sum(len(c) for c in capped):
        hair = tuple(int(args.hair[i:i + 2], 16) for i in (0, 2, 4)) if args.hair else None
        scalp(gltf, np.concatenate(capped), np.concatenate(uncapped), accessor, view, index['Head_1'], hair)
    gltf.buffers.append(gl.Buffer(byteLength=len(blob)))
    gltf.set_binary_blob(bytes(blob))
    gltf.save_binary(str(args.out))
    height = worlds['Head_1'][1, 3] - min(w[1, 3] for w in worlds.values())
    print(f'{args.out}: {len(order)} joints, {len(gltf.meshes)} meshes, head {height:.2f} m above the lowest joint')


if __name__ == '__main__':
    main()
