"""Convert the disc's per-area ambience loops into stereo OGG for an installation.

The retail loops are EA SNR/SNS pairs (5-channel EA-XMA, 48 kHz) split across
data/audio/ambienceresident.big (headers) and data/audio/ambience.big (streams).
vgmstream decodes them; ffmpeg downmixes to stereo and encodes Vorbis. Output:
<installation>/assets/private/audio/ambience/{<loop>.ogg, maps.json}.

Usage:
  python tools/prepare_ambience.py --game-root DISC --installation INSTALL_DIR \
      [--vgmstream vgmstream-cli] [--ffmpeg ffmpeg]
"""
import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.owned_game.big import BigArchive  # noqa: E402

# One loop per retail map. The disc does not name which loop each area uses
# (zone assignment is hashed), so these are picked by loop name; edit
# maps.json in the installation to change them. "*" covers custom maps.
MAP_LOOPS = {
    'University': '09_univ_campus',
    'DownTown': '04_dt_main',
    'Industrial': '11_indu_shipyard',
    'SkateSchool': '18_skate_school',
    'DownTownSkatePark': '05_dt_parks',
    'IndustrialSkatePark': '15_indu_new_factory',
    'MegaPark': '17_space_park',
    'MaloofMoneyCup': '21_interior_arena_amb',
    'BlackBoxPark': '22_interior_tunnel_amb',
    'StartPark': '06_dt_open',
    '*': '05_dt_parks',
}
# Channel order measured from the decoded streams: FL, C, FR, SL, SR.
DOWNMIX = 'pan=stereo|FL<c0+0.707*c1+0.707*c3|FR<c2+0.707*c1+0.707*c4'


def extract(archive, suffix, destination):
    names = []
    with open(archive, 'rb') as raw:
        for entry in BigArchive(archive).entries:
            if not entry.path.endswith(suffix):
                continue
            if entry.compression:
                raise RuntimeError(f'Unexpected compressed ambience entry: {entry.path}')
            raw.seek(entry.offset)
            (destination / entry.path).write_bytes(raw.read(entry.stored_size))
            names.append(Path(entry.path).stem)
    return names


def prepare(game_root, installation, vgmstream, ffmpeg, report=print, env=None):
    audio = Path(game_root) / 'data/audio'
    output = Path(installation) / 'assets/private/audio/ambience'
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temp:
        work = Path(temp)
        headers = set(extract(audio / 'ambienceresident.big', '.snr', work))
        streams = set(extract(audio / 'ambience.big', '.sns', work))
        loops = sorted(headers & streams)
        for index, name in enumerate(loops, 1):
            wav = work / (name + '.wav')
            # -i: decode one pass of the loop, not vgmstream's default 2 loops + fade.
            subprocess.run([str(vgmstream), '-i', '-o', str(wav), str(work / (name + '.snr'))],
                           check=True, stdout=subprocess.DEVNULL, env=env)
            subprocess.run([str(ffmpeg), '-v', 'error', '-y', '-i', str(wav), '-af', DOWNMIX,
                            '-c:a', 'libvorbis', '-q:a', '4', str(output / (name + '.ogg'))],
                           check=True, env=env)
            wav.unlink()
            report(f'Converted ambience {index}/{len(loops)}: {name}')
    missing = {m: l for m, l in MAP_LOOPS.items() if l not in loops}
    if missing:
        raise RuntimeError(f'Ambience loops missing from disc: {missing}')
    (output / 'maps.json').write_text(json.dumps(MAP_LOOPS, indent=2), encoding='utf-8')
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--game-root', type=Path, required=True)
    parser.add_argument('--installation', type=Path, required=True)
    parser.add_argument('--vgmstream', default='vgmstream-cli')
    parser.add_argument('--ffmpeg', default='ffmpeg')
    args = parser.parse_args()
    for tool in (args.vgmstream, args.ffmpeg):
        if not shutil.which(tool):
            raise SystemExit(f'Missing tool: {tool}')
    output = prepare(args.game_root, args.installation, args.vgmstream, args.ffmpeg,
                     lambda text: print(text, flush=True))
    print(f'Ambience ready: {output}')


if __name__ == '__main__':
    main()
