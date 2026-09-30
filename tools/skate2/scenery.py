"""Skate 2 distant scenery (mountains, terrain and bay water) as a backdrop.

The city streams only its playable cells; the hills and ocean around New San
Vanelona live in world/models/DIST_skybox.rx2 and DIST_Water.rx2. They are
written as a render-only map into native-backdrops/<map>.skate, which the
engine draws behind the world (retail_render::backdrop).

Usage:
  python -m tools.skate2.scenery <unpacked base> <map name> <target .skate>
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


def write_backdrop(base, map_name, target):
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
        manifest = dict(map_name=map_name, district_name='BAM', models=models, textures=textures_out,
                        normal_texture_policy=dict(excluded_texture_ids=[]), grind_splines=[])
        (root / 'manifest.json').write_text(json.dumps(manifest))
        target = Path(target)
        target.parent.mkdir(parents=True, exist_ok=True)
        write(root / 'manifest.json', target, None, render_only=True)
    return target


if __name__ == '__main__':
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    print('written', write_backdrop(Path(sys.argv[1]).expanduser(), sys.argv[2], Path(sys.argv[3]).expanduser()))
