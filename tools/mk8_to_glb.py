"""Convert a Mario Kart 8 model export (Collada .dae plus PNGs, as ripped with
the BFRES importer) to a static .glb the vehicle mod can load.

The exports repeat every mesh once per texture layer: the base layer (whose
albedo is a "*_Dummy_Alb" placeholder standing in for the colour variants in
Textures/), a baked-occlusion layer (`*_001`) and an emblem layer (`*_002`).
Only the base layer is kept; its placeholder albedo is replaced with the chosen
colour variant. Skin bind shapes are applied, so the model is written in its
bind pose, scaled to metres.

Usage:
  python tools/mk8_to_glb.py MODEL.dae OUT.glb [--variant A] [--scale 0.075]
"""
import argparse
import re
import struct
from pathlib import Path

import collada
import numpy as np
import pygltflib as gl


def find_file(folder: Path, name: str):
    """A file by name in the rip folder, ignoring case (rips reference
    BbMario_Alb.png for bbmario_alb.png), stray spaces ('Tire_Slk_Alb .png')
    and odd prefixes such as '/./'."""
    from urllib.parse import unquote
    name = Path(unquote(str(name)).replace('\\', '/')).name
    direct = folder / name
    if direct.is_file():
        return direct
    lower = name.lower().replace(' ', '')
    return next((p for p in folder.rglob('*') if p.name.lower().replace(' ', '') == lower), direct)


def albedo_for(image_path: Path, folder: Path, variant: str) -> Path:
    """The real albedo for a material, replacing the exporter's placeholders."""
    image_path = find_file(folder, image_path.name)
    name = image_path.name
    if 'Dummy_Alb' in name:
        textures = sorted((folder / 'Textures').glob(f'*_{variant}_Alb.png')) or sorted(folder.glob('*_Alb.png'))
        for candidate in textures:
            if 'Dummy' not in candidate.name:
                return candidate
    return find_file(folder, name)


# Driving pose for the MK8 racer rig (Skl_Root/Hip/LegL.../ArmR...): joint,
# local axis, degrees. Local Z swings a limb forward (mirrored on the right).
SEATED = [('LegL_1', 'z', 85), ('LegR_1', 'z', -85), ('KneeL_1', 'z', -80), ('KneeR_1', 'z', 80),
          ('ArmL_1', 'z', 65), ('ArmR_1', 'z', -65), ('ArmL_1', 'x', 35), ('ArmR_1', 'x', 35),
          ('ElbowL_1', 'z', 30), ('ElbowR_1', 'z', -30)]
# Collada Z-up to glTF Y-up.
Y_UP = np.array([[1, 0, 0, 0], [0, 0, 1, 0], [0, -1, 0, 0], [0, 0, 0, 1.]])


def rotation(axis, degrees):
    a = np.radians(degrees)
    c, s = np.cos(a), np.sin(a)
    m = np.eye(4)
    i, j = {'x': (1, 2), 'y': (2, 0), 'z': (0, 1)}[axis]
    m[i, i] = c; m[j, j] = c; m[i, j] = -s; m[j, i] = s
    return m


# The MK8 racer skeleton's own joints (helpers such as pupils keep their names).
SKELETON = {'Skl_Root', 'Hip', 'Spine1', 'Spine2', 'Head'} | {
    f'{part}{side}' for side in 'LR' for part in ('Leg', 'Knee', 'Foot', 'Shoulder', 'Arm', 'Elbow', 'Hand',
                                                  'Finger1', 'Finger2', 'Thumb1', 'Thumb2')}


def canon(name):
    """One joint naming across rips: some number joints per export (Head_1,
    Head_2...), others prefix them with the armature (Armature_Head)."""
    if not name or name == 'Armature':
        return name
    base = name[len('Armature_'):] if name.startswith('Armature_') else name
    stem = re.sub(r'_\d+$', '', base)
    return stem + '_1' if stem in SKELETON else base


class Rig:
    """Joint hierarchy of the visual scene, posed with extra local rotations."""

    def __init__(self, document, pose):
        self.nodes = {}

        def walk(node, parent=None):
            if type(node).__name__ == 'Node':
                self.nodes[canon(node.id)] = (parent, np.array(node.matrix, dtype=np.float64))
                parent = canon(node.id)
            for child in getattr(node, 'children', []):
                walk(child, parent)
        for node in document.scene.nodes:
            walk(node)
        self.extra = {}
        for joint, axis, degrees in pose:
            self.extra[joint] = self.extra.get(joint, np.eye(4)) @ rotation(axis, degrees)

    def world(self, name):
        parent, local = self.nodes[name]
        local = local @ self.extra.get(name, np.eye(4))
        return (self.world(parent) if parent else np.eye(4)) @ local

    def skin(self, controller, vertices):
        """Linear blend skinning of bind-shape vertices into the posed rig."""
        original = [str(n) for n in controller.weight_joints.data.reshape(-1)]
        names = [canon(n) for n in original]
        weights = controller.weights.data.reshape(-1)
        skinning = {canon(n): self.world(canon(n)) @ np.array(controller.joint_matrices[n])
                    for n in original if canon(n) in self.nodes and n in controller.joint_matrices}
        bind = np.array(controller.bind_shape_matrix).reshape(4, 4)
        homogeneous = np.c_[vertices, np.ones(len(vertices))] @ bind.T
        out = np.zeros((len(vertices), 3))
        for v, pairs in enumerate(controller.index):
            total = 0.0
            for joint, weight in pairs:
                w = weights[weight]
                out[v] += w * (skinning[names[joint]] @ homogeneous[v])[:3]
                total += w
            if total > 0:
                out[v] /= total
        return out


def primitives(document, folder: Path, variant: str, rig=None):
    """(material name, texture path, positions, normals, uvs, indices) per base-layer primitive."""
    images = {i.id: folder / i.path for i in document.images}
    skinned = {c.geometry.id for c in document.controllers}
    sources = list(document.controllers) + [g for g in document.geometries if g.id not in skinned]
    for controller_or_geometry in sources:
        geometry = getattr(controller_or_geometry, 'geometry', controller_or_geometry)
        if re.search(r'_00\d-mesh$', geometry.id):
            continue
        bind = np.array(getattr(controller_or_geometry, 'bind_shape_matrix', np.eye(4)), dtype=np.float64).reshape(4, 4)
        for primitive in geometry.primitives:
            if not hasattr(primitive, 'vertex_index'):
                continue
            material = document.materials.get(primitive.material) if hasattr(document.materials, 'get') else None
            material = material or next((m for m in document.materials if m.id == primitive.material), None)
            texture = None
            if material is not None and isinstance(material.effect.diffuse, collada.material.Map):
                surface = material.effect.diffuse.sampler.surface
                texture = albedo_for(images[surface.image.id], folder, variant)
            triangles = primitive.vertex_index.reshape(-1, 3)
            normals = primitive.normal_index.reshape(-1, 3) if primitive.normal is not None else None
            uvs = primitive.texcoord_indexset[0].reshape(-1, 3) if len(primitive.texcoordset) else None
            # De-index into flat per-corner vertices (the export's attributes use separate indices).
            corners = triangles.reshape(-1)
            if rig is not None and hasattr(controller_or_geometry, 'index'):
                # Posed (and made Y-up) as a whole; normals are recomputed below.
                posed = rig.skin(controller_or_geometry, primitive.vertex)
                positions = (np.c_[posed, np.ones(len(posed))] @ Y_UP.T)[:, :3][corners]
            else:
                positions = primitive.vertex[corners]
                positions = (np.c_[positions, np.ones(len(positions))] @ bind.T)[:, :3]
            normal = primitive.normal[normals.reshape(-1)] if normals is not None else np.tile([0, 1, 0], (len(corners), 1))
            normal = normal @ bind[:3, :3].T
            if rig is not None:
                # Flat per-face normals from the posed triangles.
                tri = positions.reshape(-1, 3, 3)
                face = np.cross(tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0])
                normal = np.repeat(face, 3, axis=0)
            normal /= np.maximum(np.linalg.norm(normal, axis=1, keepdims=True), 1e-9)
            uv = primitive.texcoordset[0][uvs.reshape(-1)] if uvs is not None else np.zeros((len(corners), 2))
            yield geometry.id, texture, positions, normal, uv


def write_glb(parts, out: Path, scale: float, node_names=None):
    gltf = gl.GLTF2(scene=0, scenes=[gl.Scene(nodes=[])], asset=gl.Asset(generator='mk8_to_glb'))
    blob = bytearray()

    def view(data: bytes, target=None):
        while len(blob) % 4:
            blob.append(0)
        gltf.bufferViews.append(gl.BufferView(buffer=0, byteOffset=len(blob), byteLength=len(data), target=target))
        blob.extend(data)
        return len(gltf.bufferViews) - 1

    textures = {}
    for name, texture, positions, normals, uvs in parts:
        positions = (positions * scale).astype(np.float32)
        normals = normals.astype(np.float32)
        # Collada V runs up; glTF V runs down.
        uvs = np.c_[uvs[:, 0], 1.0 - uvs[:, 1]].astype(np.float32)
        attributes = {}
        for key, array in (('POSITION', positions), ('NORMAL', normals), ('TEXCOORD_0', uvs)):
            gltf.accessors.append(gl.Accessor(bufferView=view(array.tobytes(), gl.ARRAY_BUFFER), componentType=gl.FLOAT,
                                              count=len(array), type=gl.VEC3 if array.shape[1] == 3 else gl.VEC2,
                                              min=array.min(0).tolist() if key == 'POSITION' else None,
                                              max=array.max(0).tolist() if key == 'POSITION' else None))
            attributes[key] = len(gltf.accessors) - 1
        material = gl.Material(name=name, pbrMetallicRoughness=gl.PbrMetallicRoughness(metallicFactor=0.0, roughnessFactor=0.6),
                               doubleSided=True)
        if texture is not None and texture.is_file():
            if texture not in textures:
                gltf.images.append(gl.Image(bufferView=view(texture.read_bytes()), mimeType='image/png'))
                gltf.textures.append(gl.Texture(source=len(gltf.images) - 1))
                textures[texture] = len(gltf.textures) - 1
            material.pbrMetallicRoughness.baseColorTexture = gl.TextureInfo(index=textures[texture])
            # Light housings glow.
            if 'Light' in name:
                material.emissiveTexture = gl.TextureInfo(index=textures[texture])
                material.emissiveFactor = [1.0, 1.0, 1.0]
        gltf.materials.append(material)
        gltf.meshes.append(gl.Mesh(name=name, primitives=[gl.Primitive(attributes=gl.Attributes(**attributes),
                                                                         material=len(gltf.materials) - 1)]))
        gltf.nodes.append(gl.Node(name=(node_names or {}).get(name, name), mesh=len(gltf.meshes) - 1))
        gltf.scenes[0].nodes.append(len(gltf.nodes) - 1)
    gltf.buffers.append(gl.Buffer(byteLength=len(blob)))
    gltf.set_binary_blob(bytes(blob))
    gltf.save_binary(str(out))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('model', type=Path)
    parser.add_argument('out', type=Path)
    parser.add_argument('--variant', default='A', help='colour variant letter from Textures/ (A-D)')
    parser.add_argument('--scale', type=float, default=0.075, help='model units to metres')
    parser.add_argument('--seated', action='store_true', help='pose an MK8 racer for driving, hips at the origin')
    args = parser.parse_args()
    document = collada.Collada(str(args.model), ignore=[collada.common.DaeUnsupportedError, collada.common.DaeBrokenRefError])
    rig = Rig(document, SEATED) if args.seated else None
    parts = list(primitives(document, args.model.parent, args.variant, rig))
    if rig is not None:
        # Hips at the origin so the driver drops onto the kart's seat point.
        hip = (Y_UP @ rig.world('Hip_1'))[:3, 3]
        parts = [(n, t, p - hip, nm, uv) for n, t, p, nm, uv in parts]
    write_glb(parts, args.out, args.scale)
    low = np.min([p[2].min(0) for p in parts], 0) * args.scale
    high = np.max([p[2].max(0) for p in parts], 0) * args.scale
    print(f'{args.out}: {len(parts)} meshes, bounds {low.round(2)}..{high.round(2)} m')


if __name__ == '__main__':
    main()
