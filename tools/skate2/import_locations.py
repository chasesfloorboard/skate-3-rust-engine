"""Import Skate 2 maps (base-game city and DLC parks) as custom locations.

Each location becomes <installation>/assets/private/custom-locations/<key>/
with the converted .skate map, location.json (title, description, photo and
retail spots) and photos; the engine lists them under Custom Locations.

Sources: an extracted Skate 2 disc (data/content/worldbam.big ...) and the
Skate 2 DLC packages as downloaded (.zip holding the Xbox 360 LIVE package),
bare LIVE packages, or directories of them.

Usage:
  python -m tools.skate2.import_locations --skate2 "~/Downloads/Skate 2" \
      --dlc ~/Downloads --installation ~/Games/Skate2Rust/data/installations/<id> [--only S2Parkade]

~/Games/Skate2Rust is a copy of the game whose installation lists these maps as
its "maps" receipt (setup.rs checks receipt sizes at startup); re-imports keep
that receipt current.
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import time
import zipfile
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
from PIL import Image

REPO = Path(__file__).resolve().parents[2]
TOOLS = REPO / 'tools'
MAP_TOOLS = TOOLS / 'vendor/university/tools/vanilla_map_extraction/tools'
for p in (REPO, TOOLS, MAP_TOOLS, TOOLS / 'vendor/utt'):
    sys.path.insert(0, str(p))

from tools.skate2 import bigf, stfs  # noqa: E402
from tools.asset_pipeline.teleports import language_table, location_records  # noqa: E402
from owned_game.refpack import decompress as refpack  # noqa: E402


@dataclass
class Spec:
    key: str
    title: str
    description: str
    package: str          # DLC display name inside the LIVE header, or 'base'
    stream: str           # world stream directory name
    start: str            # locator the location starts at
    photo: str | None = None
    # Retail city teleports: locator -> photo stem.
    spots: dict[str, str] = field(default_factory=dict)


LOCATIONS = [
    Spec('S2Parkade', 'San Van Parkade', 'Skate 2 San Van Classic Pack: the skate 1 Parkade, back in Skate 2.',
         'San Van Classic Pack', 'DLC_Parkade', 'Parkade', 'ss_dlc_parkade'),
    Spec('S2School', 'San Van School', 'Skate 2 San Van Classic Pack: the skate 1 school, back in Skate 2.',
         'San Van Classic Pack', 'DLC_School', 'SUB_School', 'ss_dlc_school'),
    Spec('S2CommunityCenter', 'San Van Community Center', 'Skate 2 San Van Classic Pack: the skate 1 Community Center.',
         'San Van Classic Pack', 'DLC_Community_Center', 'Z_Community', 'ss_dlc_communitycenter'),
    Spec('S2DyrdekFantasyPlaza', "Rob Dyrdek's Fantasy Plaza", "Skate 2 DLC: Rob Dyrdek's warehouse fantasy plaza.",
         "Dyrdek's Fantasy Park Pack", 'DLC_Rob_Fantasy_Plaza', 'D_Dyrdek_Start'),
    Spec('S2MaloofMoneyCup', 'Maloof Money Cup (Skate 2)', 'Skate 2 DLC: the 2008 Maloof Money Cup contest course.',
         'Maloof Money Cup Pack', 'DLC_MaloofMoneyCup', 'maloof_money_cup_start_locator'),
    Spec('S2SanVanelona', 'New San Vanelona', 'Skate 2: the whole city of New San Vanelona.',
         'base', 'BAM', 'Z_OT11_SkatePlaza', 'ss_ot_0_oldtown', spots={
             'Z_OT01_CityHall': 'ss_ot_1_cityhall', 'Z_OT08_Cathedral': 'ss_ot_8_cathedral',
             'Z_OT14_CentralPlaza': 'ss_ot_10_centralplaza', 'Z_OT11_SkatePlaza': 'ss_ot_5_grindplaza',
             'Z_OT13_Claw': 'ss_ot_13_tech', 'Z_OT03_CourtYard': 'ss_ot_4_railandbenches',
             'Z_OT16_OTbottom': 'ss_ot_12_outskirts', 'Z_OT02': 'ss_ot_6_bythebridge',
             'Z_DT8_Matrix': 'ss_dt_8_matrix', 'Z_DT15_Library': 'ss_dt_15_library',
             'Z_DT16': 'ss_dt_16_plazademango', 'Z_DT22_Interior': 'ss_dt_22_statue', 'Z_DT02': 'ss_dt_0_downtown',
             'Z_DT10': 'ss_dt_17_fountain', 'Z_PJ01_School': 'ss_pj_1_oliverhigh', 'Z_PJ03_Slappys': 'ss_pj_3_slappys',
             'Z_PJ04_70_sSkatepark': 'ss_pj_4_snakes', 'Z_PJ02_Residential': 'ss_pj_0_stacks',
             'Z_SVM07_MegaPark': 'ss_svm_5_megapark', 'Z_SVM01_DAMStart': 'ss_svm_3_dam', 'Z_SVM01_Top': 'ss_svm_1_peak',
             'Z_SVM14_NewSpillway': 'ss_svm_14_spillwaynew', 'Z_SVM1_OLDSpillway': 'ss_svm_15_spillwayold',
             'Z_SVM_13_Bottom': 'ss_svm_13_rockgap', 'Z_SH01_classic': 'ss_sh_0_soho', 'Z_SH03': 'ss_sh_3_wingedbench',
             'Z_SH06_CoreX': 'ss_sh_6_foundation', 'Z_UR01_UrbanResTop': 'ss_ur_0_rez', 'Z_UR04_HotelPool': 'ss_ur_5_communitypool',
             'Z_UR13_pink_motel': 'ss_ur_16_pinkypool', 'Z_UR07_lombard': 'ss_ur_5_curves', 'Z_UR11_Monument': 'ss_ur_7_wall',
             'Z_Waterfront': 'ss_wf_0_waterfront', 'A_StartWaterFront_SeaWall': 'ss_wf_8_stairset'}),
]

# More city spots without FE photos: the Mega Ramp starts, the extra retail
# starts, and every F_ freeskate spot (named from its locator, see city_extras).
CITY_EXTRAS = {
    'megaRampClassicStart_global': 'Mega Ramp', 'megaRampTechStart1': 'Mega Ramp (Tech)',
    'HipRampStart_global': 'Mega Ramp Hip', 'SideRampStart_global': 'Mega Ramp Side',
    'Z_DT4_SkatelineStart': 'Skateline', 'Z_DT8_Matrix_Skate1Dyrdek': 'The Matrix (Dyrdek)',
    'Z_PJ03_BackAlleyStart': 'Back Alley', 'Z_PJ08_StockholmBestPlaceOnEarth': 'Best Place on Earth',
    'Z_SVM_DropSomeE': 'Drop Some E', 'Z_TrainingStart': 'Training Facility',
}
F_NAMES = {'F_UR_ur': 'Urban Residential', 'F_WF_GVR': 'GVR', 'F_DT_Interior': 'Downtown Interior',
           'F_OT_Oldtown': 'Old Town', 'F_DT_MangoPlaza': 'Plaza de Mango', 'F_OT_TrainingFacility': 'Training Facility Entrance', 'F_SVM_Peak': 'Cougar Mountain Peak Top', 'F_DT_Downtown': 'Downtown Streets'}


def same_name(name):
    """Key under which "The Dam", "Dam" and "dam" (or Slappy's/Slappys) match."""
    return re.sub(r'^the ', '', re.sub(r"[^a-z0-9 ]", '', name.lower())).replace(' ', '')


def city_extras(records):
    """(locator, name) for the extra city spots present in records."""
    out = [(k, v) for k, v in CITY_EXTRAS.items()]
    for r in records:
        n = r['locator']
        if n.startswith('F_') and n.count('_') >= 2:
            words = re.sub(r'(?<=[a-z])(?=[A-Z0-9])', ' ', n.split('_', 2)[2])
            out.append((n, F_NAMES.get(n, words)))
    return out


# Readable names for the city teleports (retail names live in the FE database).
CITY_NAMES = {
    'Z_OT01_CityHall': 'City Hall', 'Z_OT08_Cathedral': 'Cathedral', 'Z_OT14_CentralPlaza': 'Central Plaza',
    'Z_OT11_SkatePlaza': 'Old Town Skate Plaza', 'Z_OT13_Claw': 'The Claw', 'Z_OT03_CourtYard': 'Courtyard',
    'Z_OT16_OTbottom': 'Old Town Outskirts', 'Z_OT02': 'By the Bridge', 'Z_DT8_Matrix': 'The Matrix',
    'Z_DT15_Library': 'Library', 'Z_DT16': 'Plaza de Mango', 'Z_DT22_Interior': 'Statue Plaza', 'Z_DT02': 'Downtown',
    'Z_DT10': 'Lake Sherwin Fountain', 'Z_PJ01_School': 'Oliver High', 'Z_PJ03_Slappys': "Slappy's",
    'Z_PJ04_70_sSkatepark': "70's Skatepark", 'Z_PJ02_Residential': 'The Stacks', 'Z_SVM07_MegaPark': "Danny's MegaPark",
    'Z_SVM01_DAMStart': 'The Dam', 'Z_SVM01_Top': 'Cougar Mountain Peak', 'Z_SVM14_NewSpillway': 'New Spillway',
    'Z_SVM1_OLDSpillway': 'Old Spillway', 'Z_SVM_13_Bottom': 'Rock Gap', 'Z_SH01_classic': 'SoHo', 'Z_SH03': 'Winged Bench',
    'Z_SH06_CoreX': 'The Foundation', 'Z_UR01_UrbanResTop': 'The Rez', 'Z_UR04_HotelPool': 'Hotel Pool',
    'Z_UR13_pink_motel': 'Pinky Pool', 'Z_UR07_lombard': 'The Curves', 'Z_UR11_Monument': 'The Wall',
    'Z_Waterfront': 'Waterfront', 'A_StartWaterFront_SeaWall': 'Sea Wall',
}


def log(text):
    print(text, flush=True)


# --------------------------------------------------------------------------- sources

def find_packages(paths):
    """{LIVE display name: package file} from zips, bare packages or directories."""
    found = {}
    work = Path(ARGS.work) / 'packages'
    candidates = []
    for path in map(Path, paths):
        candidates += sorted(path.rglob('*')) if path.is_dir() else [path]
    for path in candidates:
        if not path.is_file():
            continue
        if path.suffix.lower() == '.zip':
            with zipfile.ZipFile(path) as z:
                for info in z.infolist():
                    if info.is_dir():
                        continue
                    with z.open(info) as f:
                        if f.read(4) not in (b'LIVE', b'PIRS', b'CON '):
                            continue
                    target = work / Path(info.filename).name
                    if not target.exists() or target.stat().st_size != info.file_size:
                        target.parent.mkdir(parents=True, exist_ok=True)
                        with z.open(info) as src, target.open('wb') as dst:
                            shutil.copyfileobj(src, dst, 1 << 22)
                    name = stfs.Stfs(target).name
                    found.setdefault(name, target)
        else:
            with path.open('rb') as f:
                if f.read(4) not in (b'LIVE', b'PIRS', b'CON '):
                    continue
            found.setdefault(stfs.Stfs(path).name, path)
    return found


def unpack_package(name, package):
    """Extract a DLC package's BIG archives into work/dlc/<name>/raw."""
    out = Path(ARGS.work) / 'dlc' / re.sub(r'[^A-Za-z0-9]+', '_', name)
    raw = out / 'raw'
    if (out / 'done').exists():
        return raw
    stfs.Stfs(package).extract(out / 'package')
    for big in (out / 'package').rglob('*.big'):
        bigf.extract(big, raw)
    shutil.rmtree(out / 'package')
    (out / 'done').touch()
    return raw


def unpack_base(skate2):
    out = Path(ARGS.work) / 'base'
    if not (out / 'done').exists():
        from extract_skate2_big4 import read_entries
        for archive in ('data/content/worldbam.big',):
            for entry in read_entries(skate2 / archive):
                target = out / entry.name.replace('\\', '/')
                target.parent.mkdir(parents=True, exist_ok=True)
                with (skate2 / archive).open('rb') as src, target.open('wb') as dst:
                    src.seek(entry.offset)
                    left = entry.size
                    while left:
                        chunk = src.read(min(left, 1 << 22))
                        dst.write(chunk)
                        left -= len(chunk)
        for archive in ('data/big/globallocators.big', 'data/fe/locations.big', 'data/big/miscboot.big'):
            bigf.extract(skate2 / archive, out)
        for path in (out / 'data/content/global_locators').glob('*.rx2'):
            path.write_bytes(unpacked(path))
        (out / 'done').touch()
    return out


# --------------------------------------------------------------------------- text

def unpacked(path):
    data = path.read_bytes()
    return refpack(data, int.from_bytes(data[2:5], 'big')) if data[:2] == b'\x10\xfb' else data


def strings(root):
    """label -> English text from every language table pair under root."""
    text = {}
    for english in (root / 'data/fe/languages/english').glob('*.BIN'):
        if 'HISTOGRAM' in english.name:
            continue
        labels = root / 'data/fe/languages/labels' / english.name.replace('English', 'Labels')
        if not labels.exists():
            continue
        try:
            lab, eng = language_table(unpacked(labels)), language_table(unpacked(english))
        except ValueError:
            continue  # Skate 2 base tables use another layout; DLC ones are readable.
        for key, label in lab.items():
            if key in eng:
                text[label.decode('latin-1').strip()] = eng[key].decode('latin-1').replace('\xab', 'skate.').strip()
    return text


def challenge_text(locator, text):
    x = re.sub(r'_(challenge_?)?(challenge)?locator\d*$', '', locator, flags=re.I).upper()
    for base in (x, x.replace('SPOTFILM', 'FILM'), x.replace('SPOTPHOTO', 'PHOTO')):
        names = [f'ID_CHALLENGE_{base}', f'ID_DLC_CHALLENGE_{base}']
        title = next((text[n + s] for n in names for s in ('', '_TITLE') if n + s in text), None)
        if title:
            desc = next((text[n + s] for n in names for s in ('_DESC', '_DESCRIPTION') if n + s in text), None)
            return title, desc
    return None, None


# --------------------------------------------------------------------------- conversion

def convert(spec, stream, stage, start, bam_textures, props):
    from prepare_hawaiian_dream import prepare
    from prepare_university import EXCLUDED_NORMAL_TEXTURE_IDS
    from build_retail_collision_archive import build_archive
    from build_skate2_lightmap_aliases import _raw_lightmap_groups
    from tools.asset_pipeline.map_writer import write as write_map, SpawnSelector
    work = Path(ARGS.work) / 'convert' / spec.key
    extra, excluded = {}, EXCLUDED_NORMAL_TEXTURE_IDS
    if spec.stream == 'BAM':
        import prepare_skate2_bam as bam
        extra = dict(grind_coordinate_mode='mixed_cell_local',
                     excluded_static_model_asset_ids=bam.EXCLUDED_STATIC_MODEL_ASSET_IDS)
        excluded = excluded + bam.EXCLUDED_NORMAL_TEXTURE_IDS
    started = time.time()
    manifest = work / 'intermediate/manifest.json'
    if not manifest.exists():
        manifest = prepare(stream_directory=stream, output_root=work / 'intermediate', utt_root=TOOLS / 'vendor/utt',
                           district_name=spec.stream, map_name=spec.key, package_name='Skate 2',
                           cache_format='skate3-rust-map-v1',
                           texture_stream_names=('Tex',) if any(stream.glob('cTex_*.xsf')) else (),
                           excluded_normal_texture_ids=excluded, raw_texture_cache=True,
                           collision_consumer=SpawnSelector(spec.stream).consider, write_render_sources=True,
                           allow_material_import_order_fallback=True, **extra)
    log(f'{spec.key}: prepared in {time.time() - started:.0f}s')
    root = manifest.parent
    pristine = root / 'manifest.pristine.json'
    if not pristine.exists():
        shutil.copyfile(manifest, pristine)
    m = json.loads(pristine.read_text())

    # DLC materials share base-game textures: take them from the city's cache
    # when it is converted, otherwise drop the (few) references.
    have, borrowed, dropped = set(m['textures']), 0, 0
    for model in m['models']:
        for mesh in model['meshes']:
            refs = [('texture_id', mesh.get('texture_id'))] + list(mesh.get('retail_texture_ids', {}).items())
            for role, tid in refs:
                if not tid or tid in have:
                    continue
                source = bam_textures.get(tid) if bam_textures else None
                if source:
                    entry, source_root = source
                    target = f'textures/borrowed_{tid}.rgba'
                    shutil.copyfile(source_root / entry['rgba'], root / target)
                    m['textures'][tid] = dict(entry, rgba=target, variants=[], alternate_stream_assets=[])
                    have.add(tid); borrowed += 1
                elif role == 'texture_id':
                    mesh['texture_id'] = None; dropped += 1
                else:
                    mesh['retail_texture_ids'].pop(role); dropped += 1
    log(f'{spec.key}: {borrowed} shared textures from the city, {dropped} references dropped')

    # Skate 2 lightmap pages pack three lightmaps into R/G/B (the material's
    # "component") and keep colour in a low-res chromaticity page. The engine
    # expects one RGB lightmap, so bake page[component] * chroma per mesh.
    def pixels(tid):
        e = m['textures'][tid]
        return np.fromfile(root / e['rgba'], np.uint8).reshape(e['height'], e['width'], 4)
    baked = {}
    for model in m['models']:
        groups = _raw_lightmap_groups((root / model['rx2']).read_bytes())
        for mesh in model['meshes']:
            roles = mesh.get('retail_texture_ids', {})
            lm, gi = roles.get('lightmap'), mesh.get('retail_material_group_index', -1)
            comp = groups[gi].get('lightmap_component') if 0 <= gi < len(groups) else None
            if not lm or comp is None or lm.startswith('s2lm_'):
                continue
            key = (lm, comp, roles.get('chromaticity') if roles.get('chromaticity') in m['textures'] else None)
            if key not in baked:
                page = pixels(lm)
                h, w = page.shape[:2]
                light = page[..., comp].astype(np.float32) / 255
                colour = np.ones((h, w, 3), np.float32)
                if key[2]:
                    c = np.asarray(Image.fromarray(np.ascontiguousarray(pixels(key[2])[..., :3])).resize((w, h), Image.BILINEAR),
                                   np.float32) / 255
                    luma = c @ np.array([.2126, .7152, .0722], np.float32)
                    colour = np.clip(c / np.maximum(luma, 1e-3)[..., None], 0, 3)
                # The shader squares the lightmap: store L*sqrt(colour) so light = L^2 * colour.
                rgb = np.clip(light[..., None] * np.sqrt(colour) * 255, 0, 255).astype(np.uint8)
                if spec.stream == 'BAM' and min(w, h) >= 64:
                    # The city's unpacked lightmaps would need ~2.4 GiB: half
                    # resolution keeps loading within memory, and they are soft.
                    w, h = w // 2, h // 2
                    rgb = np.asarray(Image.fromarray(rgb).resize((w, h), Image.BOX))
                tid = f's2lm_{len(baked)}'
                np.concatenate([rgb, np.full((h, w, 1), 255, np.uint8)], 2).tofile(root / f'textures/{tid}.rgba')
                m['textures'][tid] = dict(rgba=f'textures/{tid}.rgba', width=w, height=h, format='baked', warnings=[])
                baked[key] = tid
            roles['lightmap'] = baked[key]
    if m['grind_coordinate_policy']['mode'] == 'mixed_cell_local':
        # The city mixes world-space and cell-local rails, even within one
        # asset. Move cell-local rails into world space and leave out the few
        # that cannot be told apart, so the engine gets world-space rails only.
        from retail_grind_splines import (classify_grind_coordinate_frame, grind_cell_translation,
                                          translate_native_segment_payload)
        kept, moved = [], 0
        for rail in m['grind_splines']:
            frame = classify_grind_coordinate_frame(rail['stream_file'], rail['native_segment_payloads'])
            if frame == 'cell_local':
                offset = grind_cell_translation(rail['stream_file'])
                rail['native_segment_payloads'] = [translate_native_segment_payload(p, offset)
                                                   for p in rail['native_segment_payloads']]
                moved += 1
            elif frame == 'ambiguous':
                continue
            kept.append(rail)
        log(f'{spec.key}: {len(kept)} grind rails ({moved} moved to world space, '
            f'{len(m["grind_splines"]) - len(kept)} ambiguous left out)')
        m['grind_splines'] = kept
        m['grind_coordinate_policy']['mode'] = 'world_space'
    # Chroma is baked into the lightmaps (the engine never samples it), and the
    # map writer stores every manifest texture: keep only referenced ones.
    used = set()
    for model in m['models']:
        for mesh in model['meshes']:
            mesh.get('retail_texture_ids', {}).pop('chromaticity', None)
            used.update(t for t in [mesh.get('texture_id'), *mesh.get('retail_texture_ids', {}).values()] if t)
    before = len(m['textures'])
    m['textures'] = {k: v for k, v in m['textures'].items() if k in used}
    for entry in m['textures'].values():
        entry.pop('variants', None); entry.pop('alternate_stream_assets', None)
    log(f'{spec.key}: {len(m["textures"])} of {before} textures used')
    manifest.write_text(json.dumps(m))
    log(f'{spec.key}: baked {len(baked)} lightmaps')

    if props:
        from tools.asset_pipeline.dynamic_props import export
        catalog, cache = props
        target = ARGS.installation.expanduser() / 'assets/private/native-props' / f'{spec.key}.skate'
        target.parent.mkdir(parents=True, exist_ok=True)
        placed, unresolved = export(manifest, [cache], target, catalog_path=catalog)
        log(f'{spec.key}: {placed} props placed, {unresolved} without a Skate 2 template')
    collision = work / 'collision.rwcmset'
    build_archive(manifest, collision)
    final = stage / f'{spec.key}.skate'
    write_map(manifest, final, collision, log, prepared_spawn=(start[0], start[1] + 1.0, start[2]))
    log(f'{spec.key}: map written in {time.time() - started:.0f}s')
    return m, root


def prop_catalog(skate2):
    """Skate 2's movable-object templates (worldmisc.big world/dmo/DMO)."""
    from prepare_hawaiian_dream import prepare
    from tools.asset_pipeline.dynamic_props import save_catalog
    work = Path(ARGS.work) / 'dmo'
    path, cache = work / 'catalog.json', work / 'cache/DMO'
    if path.exists():
        return path, cache
    bigf.extract(skate2 / 'data/content/worldmisc.big', work / 'raw')
    prepare(stream_directory=work / 'raw/data/content/world/dmo/DMO', output_root=cache, utt_root=TOOLS / 'vendor/utt',
            district_name='DMO', map_name='DMO', raw_texture_cache=True, allow_material_import_order_fallback=True)
    save_catalog([cache], path)
    # Placements bind texture GUIDs in the high-bit DMO namespace; Skate 2's
    # stream keys omit that bit. Alias each texture under both keys.
    data = json.loads(path.read_text())
    for key, value in list(data['textures'].items()):
        data['textures'][f'0x{int(key, 16) | 1 << 63:016x}'] = value
    path.write_text(json.dumps(data))
    return path, cache


def photo(rx2, target):
    import rx2_parser
    texture = rx2_parser.parse_rx2(unpacked(rx2)).textures[0]
    Image.frombytes('RGBA', (texture.width, texture.height), texture.rgba).save(target)


def screenshot(game, installation, map_path, target):
    """Photo for a location without retail artwork: the view at its start."""
    import subprocess
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        shot = Path(tmp) / 'shot.png'
        env = dict(__import__('os').environ, SKATE_VERIFY_DELAY='8', SKATE_DEBUG_HOUR='13')
        subprocess.run([game, '--assets', installation / 'assets', '--map', map_path, '--verify', shot],
                       env=env, timeout=180, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not shot.exists():
            log(f'Screenshot failed: {map_path.name}')
            return False
        image = Image.open(shot).convert('RGB')
        w, h = image.size
        crop = image.crop((0, (h - w // 2) // 2, w, (h - w // 2) // 2 + w // 2)) if h > w // 2 else image
        crop.resize((512, 256), Image.LANCZOS).save(target)
    return True


# --------------------------------------------------------------------------- main

def main():
    global ARGS
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--skate2', type=Path, help='Extracted Skate 2 disc (for the city and shared textures)')
    parser.add_argument('--dlc', type=Path, nargs='*', default=[], help='DLC zips, LIVE packages or directories')
    parser.add_argument('--installation', type=Path, required=True)
    parser.add_argument('--work', type=Path, default=Path.home() / '.cache/skate3rust-skate2')
    parser.add_argument('--game', type=Path, help='skate3rust binary, for photos of locations without artwork')
    parser.add_argument('--sky', default='University', help='Installed sky copied for each location')
    parser.add_argument('--only', nargs='*', help='Location keys to import')
    parser.add_argument('--keep-cache', action='store_true', help='Keep the prepared city for faster re-imports')
    parser.add_argument('--spots-only', action='store_true', help='Only refresh installed locations\' spot lists')
    ARGS = parser.parse_args()
    installation = ARGS.installation.expanduser()
    assets = installation / 'assets'
    target_root = assets / 'private/custom-locations'
    packages = find_packages([p.expanduser() for p in ARGS.dlc])
    log('DLC packages: ' + (', '.join(sorted(packages)) or 'none'))
    base = unpack_base(ARGS.skate2.expanduser()) if ARGS.skate2 else None
    if ARGS.skate2 and not ARGS.spots_only:
        from skate2.physics import build as build_physics
        build_physics(ARGS.skate2.expanduser(), installation, log)
    city_text = strings(base) if base else {}

    props = prop_catalog(ARGS.skate2.expanduser()) if ARGS.skate2 else None
    specs = [s for s in LOCATIONS if not ARGS.only or s.key in ARGS.only]
    # The city first: DLC maps borrow its shared textures.
    specs.sort(key=lambda s: s.stream != 'BAM')
    bam_textures = None
    for spec in specs:
        if spec.package == 'base':
            if not base:
                log(f'{spec.key}: needs --skate2')
                continue
            raw = base
        elif spec.package in packages:
            raw = unpack_package(spec.package, packages[spec.package])
        else:
            log(f'{spec.key}: package "{spec.package}" not found')
            continue
        stream = raw / 'data/content/world/stream' / spec.stream
        records = []
        for path in sorted((raw / 'data/content/global_locators').glob('*.rx2')):
            records += location_records(path.read_bytes())
        by_name = {r['locator']: r for r in records}
        if spec.start not in by_name:
            log(f'{spec.key}: start locator {spec.start} missing')
            continue
        start = by_name[spec.start]['matrix'][3][:3]
        text = dict(city_text, **strings(raw))
        if ARGS.spots_only:
            refresh_spots(target_root / spec.key, spec, by_name)
            continue

        stage = Path(ARGS.work) / 'stage' / spec.key
        if stage.exists():
            shutil.rmtree(stage)
        stage.mkdir(parents=True)
        m, root = convert(spec, stream, stage, start, bam_textures, props)
        if spec.stream == 'BAM':
            bam_textures = {tid: (e, root) for tid, e in m['textures'].items() if 'rgba' in e}

        # Spots: retail teleports for the city, named challenge spots for DLC.
        # A pack's locator files can cover several maps: keep this map's.
        bounds = [c['bounds'] for a in m['simulation_assets'] for c in a['collision_meshes']]
        lo = np.min([b['minimum'] for b in bounds], 0) - 5
        hi = np.max([b['maximum'] for b in bounds], 0) + 5
        records = [r for r in records if np.all((lo <= r['matrix'][3][:3]) & (r['matrix'][3][:3] <= hi))]
        spots, seen = [], set()
        def add(locator, name, description=None, image=None):
            r = by_name[locator]
            where = tuple(round(v) for v in r['matrix'][3][:3])
            if where in seen:
                return
            seen.add(where)
            spots.append(dict(id=re.sub(r'[^A-Za-z0-9_]+', '_', locator), name=name, matrix=r['matrix'],
                              **({'description': description} if description else {}), **({'image': image} if image else {})))
        add(spec.start, 'Start')
        photos = raw / 'data/fe/source/images/locations'
        for locator, stem in spec.spots.items():
            if locator in by_name:
                image = None
                if (photos / f'{stem}.rx2').exists():
                    photo(photos / f'{stem}.rx2', stage / f'{stem}.png')
                    image = f'{stem}.png'
                add(locator, CITY_NAMES.get(locator, locator), image=image)
        if spec.stream == 'BAM':
            names = {same_name(sp['name']) for sp in spots}
            for locator, name in city_extras(records):
                if locator in by_name and same_name(name) not in names:
                    names.add(same_name(name))
                    add(locator, name)
        if not spec.spots:
            for r in records:
                name, description = challenge_text(r['locator'], text)
                if name:
                    add(r['locator'], name, description)
        spots = spots[:1] + sorted(spots[1:], key=lambda s: s['name'].lower())

        location = dict(version=1, game='skate2', title=spec.title, description=spec.description, map=f'{spec.key}.skate', destinations=spots)
        if spec.photo and (photos / f'{spec.photo}.rx2').exists():
            photo(photos / f'{spec.photo}.rx2', stage / 'preview.png')
            location['image'] = 'preview.png'
        (stage / 'location.json').write_text(json.dumps(location, indent=1))

        destination = target_root / spec.key
        if destination.exists():
            shutil.rmtree(destination)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.move(stage, destination)
        if 'image' not in location and ARGS.game:
            if screenshot(ARGS.game, installation, destination / f'{spec.key}.skate', destination / 'preview.png'):
                location['image'] = 'preview.png'
                (destination / 'location.json').write_text(json.dumps(location, indent=1))
        if spec.stream == 'BAM':
            from tools.skate2.scenery import write_backdrop
            write_backdrop(raw, spec.key, assets / 'private/native-backdrops' / f'{spec.key}.skate')
        skies = assets / 'private/native-skies'
        for suffix in ('.json', '.rgba', '.sun.rgba'):
            if (skies / f'{ARGS.sky}{suffix}').exists():
                shutil.copyfile(skies / f'{ARGS.sky}{suffix}', skies / f'{spec.key}{suffix}')
        log(f'{spec.key}: installed with {len(spots)} spots')
        if spec.stream != 'BAM' and not ARGS.keep_cache:
            shutil.rmtree(Path(ARGS.work) / 'convert' / spec.key, ignore_errors=True)
    refresh_receipt(installation)
    if not ARGS.keep_cache:
        shutil.rmtree(Path(ARGS.work) / 'convert', ignore_errors=True)


def refresh_spots(folder, spec, by_name):
    """Add the extra city spots to an installed location.json in place."""
    path = folder / 'location.json'
    if spec.stream != 'BAM' or not path.exists():
        return
    location = json.loads(path.read_text())
    spots = location['destinations']
    seen = {tuple(round(v) for v in s['matrix'][3][:3]) for s in spots}
    names = {same_name(s['name']) for s in spots}
    for locator, name in city_extras(list(by_name.values())):
        r = by_name.get(locator)
        if not r or same_name(name) in names:
            continue
        names.add(same_name(name))
        where = tuple(round(v) for v in r['matrix'][3][:3])
        if where in seen:
            continue
        seen.add(where)
        spots.append(dict(id=re.sub(r'[^A-Za-z0-9_]+', '_', locator), name=name, matrix=r['matrix']))
    location['destinations'] = spots[:1] + sorted(spots[1:], key=lambda s: s['name'].lower())
    path.write_text(json.dumps(location, indent=1))
    log(f'{spec.key}: {len(spots)} spots')


def refresh_receipt(installation):
    """Keep a Skate 2 copy's maps receipt matching the re-imported files."""
    marker = installation.parent.parent / 'installation.json'
    if not marker.exists():
        return
    data = json.loads(marker.read_text())
    maps = data.get('outputs', {}).get('maps', {})
    if not maps or not all(k.startswith('assets/private/custom-locations/') for k in maps):
        return  # A Skate 3 installation: its receipt covers the retail districts.
    data['outputs']['maps'] = {str(p.relative_to(installation)): {'size': p.stat().st_size}
                               for p in sorted((installation / 'assets/private/custom-locations').glob('*/*.skate'))}
    marker.write_text(json.dumps(data))


if __name__ == '__main__':
    main()
