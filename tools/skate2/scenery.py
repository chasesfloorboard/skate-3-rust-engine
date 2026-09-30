"""Skate 2 distant scenery (mountains, terrain and bay water) as a backdrop.

The city streams only its playable cells; the hills and ocean around New San
Vanelona live in world/models/DIST_skybox.rx2 and DIST_Water.rx2. They are
written as a render-only map into native-backdrops/<map>.skate, which the
engine draws behind the world (retail_render::backdrop).

Blocks that Skate 2 draws only from its proxy (low-detail) stream are added
too when the prepared city and proxy manifests are given.

Usage:
  python -m tools.skate2.scenery <unpacked base> <map name> <target .skate> [<city manifest> <proxy manifest>]
"""
from __future__ import annotations

import hashlib
import json
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
for p in (ROOT, ROOT / 'tools', ROOT / 'tools/vendor/utt',
          ROOT / 'tools/vendor/university/tools/vanilla_map_extraction/tools'):
    if str(p) not in sys.path:
        sys.path.insert(0, str(p))

# Mesh 0 of the skybox model is the sky dome: the engine draws its own sky.
SOURCES = [('DIST_skybox', {0}), ('DIST_Water', set())]


def _keys(points, voxel):
    import numpy as np
    q = np.floor(points / voxel).astype(np.int64) + (1 << 20)
    return (q[:, 0] << 42) | (q[:, 1] << 21) | q[:, 2]


def _surface_points(tris, voxel):
    """Points covering each triangle at roughly voxel spacing."""
    import numpy as np
    a, b, c = tris[:, 0], tris[:, 1], tris[:, 2]
    steps = np.clip(np.ceil(np.maximum(np.linalg.norm(b - a, axis=1), np.linalg.norm(c - a, axis=1)) / voxel), 1, 64).astype(int)
    for n in np.unique(steps):
        sel = steps == n
        u, v = np.meshgrid(np.linspace(0, 1, n + 1), np.linspace(0, 1, n + 1))
        keep = (u + v) <= 1
        u, v = u[keep], v[keep]
        yield (a[sel, None] + (b - a)[sel, None] * u[None, :, None] + (c - a)[sel, None] * v[None, :, None]).reshape(-1, 3)


def detail_voxels(city_manifest, voxel):
    """Sorted voxel keys touched by the city's full-detail render geometry."""
    import numpy as np
    city_manifest = Path(city_manifest)
    manifest = json.loads(city_manifest.read_text())
    chunks = []
    for model in manifest['models']:
        arrays = np.load(city_manifest.parent / model['npz'])
        for name in arrays.files:
            if name.startswith('vertices_'):
                tris = arrays[name][arrays['faces_' + name[9:]]]
                chunks += [np.unique(_keys(p, voxel)) for p in _surface_points(tris, voxel)]
    return np.unique(np.concatenate(chunks))


def _subdivide(tris, uvs, normals, longest):
    """Split triangles (with their corner attributes) until no edge exceeds longest."""
    import numpy as np
    done = [], [], []
    while len(tris):
        edge = np.max(np.stack([np.linalg.norm(tris[:, i] - tris[:, (i + 1) % 3], axis=1) for i in range(3)]), 0)
        small = edge <= longest
        for out, arr in zip(done, (tris, uvs, normals)):
            out.append(arr[small])
        tris, uvs, normals = tris[~small], uvs[~small], normals[~small]
        if not len(tris):
            break
        def split(arr):
            m01, m12, m20 = (arr[:, 0] + arr[:, 1]) / 2, (arr[:, 1] + arr[:, 2]) / 2, (arr[:, 2] + arr[:, 0]) / 2
            return np.concatenate([np.stack(t, 1) for t in ((arr[:, 0], m01, m20), (m01, arr[:, 1], m12),
                                                             (m20, m12, arr[:, 2]), (m01, m12, m20))])
        tris, uvs, normals = split(tris), split(uvs), split(normals)
    return tuple(np.concatenate(d) for d in done)


def proxy_fill(proxy_manifest, detail, voxel=1.0, reach=2, longest=3.0):
    """Proxy (low-detail) city meshes, kept only where no full-detail geometry
    lies within reach voxels: whole blocks exist only in the proxy stream."""
    import numpy as np
    proxy_manifest = Path(proxy_manifest)
    manifest = json.loads(proxy_manifest.read_text())
    r = range(-reach, reach + 1)
    offsets = np.array([(dx << 42) + (dy << 21) + dz for dx in r for dy in r for dz in r], np.int64)
    for model in manifest['models']:
        arrays = np.load(proxy_manifest.parent / model['npz'])
        out, meshes = {}, []
        for mesh in model['meshes']:
            i = mesh['index']
            f = arrays[f'faces_{i}']
            tris, uvs, normals = _subdivide(arrays[f'vertices_{i}'][f], arrays[f'uvs_{i}'][f], arrays[f'normals_{i}'][f], longest)
            keys = _keys(tris.mean(1), voxel)
            near = np.zeros(len(keys), bool)
            for o in offsets:
                k = keys + o
                j = np.clip(np.searchsorted(detail, k), 0, len(detail) - 1)
                near |= detail[j] == k
            tris, uvs, normals = tris[~near], uvs[~near], normals[~near]
            if not len(tris):
                continue
            out[f'vertices_{i}'] = tris.reshape(-1, 3).astype(np.float32)
            out[f'uvs_{i}'] = uvs.reshape(-1, 2).astype(np.float32)
            out[f'normals_{i}'] = normals.reshape(-1, 3).astype(np.float32)
            out[f'faces_{i}'] = np.arange(len(tris) * 3, dtype=np.uint32).reshape(-1, 3)
            meshes.append(dict(mesh, vertex_count=len(tris) * 3, triangle_count=len(tris)))
        if meshes:
            yield model, meshes, out


def write_backdrop(base, map_name, target, city=None, proxy=None):
    """city/proxy: prepared manifests of the full-detail city and its proxy
    stream; with both, proxy-only blocks are added."""
    import numpy as np
    import mdl_parser
    import rx2_parser
    import prepare_hawaiian_dream as prep
    from retail_texture_decode import B5G6R5_FORMAT_ID, decode_b5g6r5
    from tools.asset_pipeline.backdrop import texture_groups
    from tools.asset_pipeline.map_writer import write
    from tools.asset_pipeline.sky import _texture
    from tools.skate2.import_locations import unpacked

    models_dir = Path(base) / 'data/content/world/models'
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        models, textures_out = [], {}
        for name, skip in SOURCES:
            raw = unpacked(models_dir / f'{name}.rx2')
            traw = unpacked(models_dir / f'{name}_Textures.rx2')
            model = mdl_parser.parse_rx2(raw, strict=False)
            groups, bindings = prep._bind_material_groups_by_guid(
                raw, prep._group_material_parameters(model.materials), len(model.meshes),
                allow_import_order_fallback=True)
            textures = rx2_parser.parse_rx2(traw)
            table = rx2_parser.RX2File(raw)
            table.parse()
            channel = texture_groups(raw, table, prep.RETAIL_TEXTURE_CHANNELS)
            arrays, meshes = {}, []
            for i, mesh in enumerate(model.meshes):
                if i in skip:
                    continue
                material = prep._material_metadata(groups, i)
                b = bindings[i]
                roles = {}
                for role, tid in material['retail_texture_ids'].items():
                    if role in ('lightmap', 'chromaticity'):
                        continue  # Packed Skate 2 lightmap pages; the backdrop is lit dynamically.
                    guid = channel[b['group_index']].get(role)
                    if guid is None:
                        continue
                    try:
                        t = _texture(textures, guid)
                    except ValueError:
                        continue
                    rgba = t.rgba
                    if t.fmt_id == B5G6R5_FORMAT_ID:
                        rgba = decode_b5g6r5(textures.data[t.data_offset:t.data_offset + t.buffer_size], t.width, t.height)
                    key = f'{name}_{tid}'
                    if key not in textures_out:
                        (root / f'{key}.rgba').write_bytes(rgba)
                        textures_out[key] = dict(width=t.width, height=t.height, rgba=f'{key}.rgba')
                    roles[role] = key
                material = dict(material, retail_texture_ids=roles,
                                texture_id=roles.get('diffuse', roles.get('transparent')))
                meshes.append(dict(material, index=i, name=groups[i]['Name'][0],
                                   retail_material_guid=f"0x{b['material_guid']:016X}",
                                   retail_material_handle=f"0x{b['material_handle']:08X}",
                                   retail_material_group_index=b['group_index'],
                                   source_offsets=mesh.source_offsets))
                arrays[f'vertices_{i}'] = mesh.vertices
                arrays[f'faces_{i}'] = mesh.faces
                arrays[f'uvs_{i}'] = mesh.uvs
                arrays[f'normals_{i}'] = mesh.normals
            np.savez(root / f'{name}.npz', **arrays)
            models.append(dict(asset_id='0x' + hashlib.sha256(raw).hexdigest()[:16], npz=f'{name}.npz', meshes=meshes))
        if city and proxy:
            detail = detail_voxels(city, 1.0)
            proxy_textures = json.loads(Path(proxy).read_text())['textures']
            for model, meshes, arrays in proxy_fill(proxy, detail):
                for mesh in meshes:
                    tid = mesh.get('texture_id')
                    if tid and tid not in textures_out:
                        e = proxy_textures[tid]
                        (root / f'proxy_{tid}.rgba').symlink_to(Path(proxy).parent / e['rgba'])
                        textures_out[tid] = dict(width=e['width'], height=e['height'], rgba=f'proxy_{tid}.rgba')
                npz = f"proxy_{model['asset_id']}.npz"
                np.savez(root / npz, **arrays)
                models.append(dict(asset_id=model['asset_id'], npz=npz, meshes=meshes))
        manifest = dict(map_name=map_name, district_name='BAM', models=models, textures=textures_out,
                        normal_texture_policy=dict(excluded_texture_ids=[]), grind_splines=[])
        (root / 'manifest.json').write_text(json.dumps(manifest))
        target = Path(target)
        target.parent.mkdir(parents=True, exist_ok=True)
        write(root / 'manifest.json', target, None, render_only=True)
    return target


if __name__ == '__main__':
    if len(sys.argv) not in (4, 6):
        sys.exit(__doc__)
    extra = [Path(a).expanduser() for a in sys.argv[4:]] or [None, None]
    print('written', write_backdrop(Path(sys.argv[1]).expanduser(), sys.argv[2], Path(sys.argv[3]).expanduser(), *extra))
