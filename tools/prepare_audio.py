"""Convert the disc's skateboard sound banks and iPod soundtrack for an installation.

Board: every clip of the banks below becomes <bank>/<n>.ogg, and board.json maps
engine events (rolling, pop, land, grinds, bails) to clips. The retail banks are
unnamed; the mapping is a best guess by bank, length and position and is meant
to be edited in the installation (no reconversion needed).

Music: ipod.mpf holds 46 licensed songs as 30 consecutive stream segments each,
in playlist order (verified: segments join sample-continuously within a song and
the 30th ends in silence). Per-song gain comes from the ipod_playlist VLT record.
Song titles are hashed IDs that are not resolved yet, so tracks are numbered.

Usage:
  python tools/prepare_audio.py --game-root DISC --installation INSTALL_DIR \
      [--vgmstream vgmstream-cli] [--ffmpeg ffmpeg] [--skip-music]
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.owned_game.big import BigArchive  # noqa: E402

BOARD_BANKS = ('PatchBank_Rolling_Surfaces.abk', 'Rolling_Rattles.abk', 'GRINDS.abk',
               'board_scrapes.abk', 'WHEEL_SKID_BANK.abk', 'Bodyslide.abk',
               'Sk8_Air_Flip_Tricks.abk', 'Seams_Bank.abk', 'Brd_Squeaks.abk',
               'sense_of_speed.abk', 'Foley_Cloth.abk', 'FOOT_DRAG.abk',
               'fstep_skateshoe1_sm.abk')

def clips(bank, numbers):
    return [f'{bank}/{n}.ogg' for n in numbers]

# Event -> clips. Loops play continuously and are faded by the engine; one-shots
# pick a random clip per event. Volumes are linear.
BOARD_MAP = {
    'roll': {'clips': clips('PatchBank_Rolling_Surfaces', [2]), 'volume': 0.9},
    'rattle': {'clips': clips('Rolling_Rattles', [1]), 'volume': 0.35},
    'grind_trucks': {'clips': clips('GRINDS', [1]), 'volume': 0.9},
    'grind_board': {'clips': clips('GRINDS', [2]), 'volume': 0.9},
    'bodyslide': {'clips': clips('Bodyslide', [1]), 'volume': 0.7},
    'pop': {'clips': clips('board_scrapes', range(1, 31)), 'volume': 0.9},
    'land': {'clips': clips('PatchBank_Rolling_Surfaces', range(8, 17)), 'volume': 1.0},
    'grind_enter': {'clips': clips('PatchBank_Rolling_Surfaces', range(8, 17)), 'volume': 0.8},
    'powerslide': {'clips': clips('WHEEL_SKID_BANK', range(1, 97)), 'volume': 0.6},
    'bail': {'clips': clips('Bodyslide', [2, 3, 4, 5, 7, 8, 9, 10]), 'volume': 1.0},
    'flip': {'clips': clips('Sk8_Air_Flip_Tricks', range(18, 31)), 'volume': 0.4},
}

# Body impact layers from the SPLC banks (tools/ea_bank.py), by sample order.
# The banks are unnamed; picks come from acoustic profiling:
# - Skate_Collisions: the dull, bass-heavy thuds, light / medium / heavy by
#   length and depth (other runs are brighter board and object hits);
# - HOM_Set_1 (Hall of Meat): the non-voiced crunches, a fleshy layer;
# - Skate_Metal: the heaviest, lowest metal impacts, for bodies on metal;
# - HOM_Set_2: the shortest, brightest non-voiced cracks, as bone breaks.
BODY_SOURCES = {
    'Skate_Collisions.bnk': 'Body_Impacts',
    'HOM_Set_1.bnk': 'Body_Flesh',
    'Skate_Metal.bnk': 'Body_Metal',
    'HOM_Set_2.bnk': 'Body_Bones',
}
BODY_MAP = {
    'body_light': {'clips': clips('Body_Impacts', range(1, 13)), 'volume': 0.9},
    'body_medium': {'clips': clips('Body_Impacts', [16, 17, 18, 20, 21, 23, 24, 26, 27, 28, 29, 30]), 'volume': 1.0},
    'body_heavy': {'clips': clips('Body_Impacts', [36, 37, 38, 39, 43, 44, 45, 46, 47, 48]), 'volume': 1.0},
    'body_flesh': {'clips': clips('Body_Flesh', [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 14, 15, 16, 18, 20, 21, 23, 25,
                                                  26, 27, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 44, 47, 52, 53, 55]), 'volume': 0.7},
    'body_metal': {'clips': clips('Body_Metal', [5, 6, 7, 29, 30, 51, 52, 54, 129, 189, 260, 267, 268, 273]), 'volume': 0.9},
    'body_bone': {'clips': clips('Body_Bones', [8, 14, 15, 30, 37, 49, 52]), 'volume': 0.9},
}

SEGMENTS_PER_SONG = 30


def stream_count(vgmstream, path, env):
    out = subprocess.run([str(vgmstream), '-m', str(path)], capture_output=True, text=True,
                         check=True, env=env).stdout
    return int(next(l.split(':')[1] for l in out.splitlines() if l.startswith('stream count')))


def decode(vgmstream, source, index, wav, env):
    # -i: one pass without vgmstream's loop repetition and fade.
    subprocess.run([str(vgmstream), '-i', '-s', str(index), '-o', str(wav), str(source)],
                   check=True, stdout=subprocess.DEVNULL, env=env)


def encode(ffmpeg, inputs, output, env, quality=4, concat_list=None):
    command = [str(ffmpeg), '-v', 'error', '-y']
    if concat_list:
        command += ['-f', 'concat', '-safe', '0', '-i', str(concat_list)]
    else:
        command += ['-i', str(inputs)]
    command += ['-c:a', 'libvorbis', '-q:a', str(quality), str(output)]
    subprocess.run(command, check=True, env=env)


def workers():
    return max(1, min(8, os.cpu_count() or 1))


def prepare_board(game_root, installation, vgmstream, ffmpeg, report=print, env=None):
    output = Path(installation) / 'assets/private/audio/board'
    output.mkdir(parents=True, exist_ok=True)
    # Fresh clips: tools/seamless_loops.py must process them again.
    (output / '.seamless.json').unlink(missing_ok=True)
    archive = Path(game_root) / 'data/audio/audiofiles.big'
    with tempfile.TemporaryDirectory() as temp, open(archive, 'rb') as raw:
        work = Path(temp)
        wanted = {e.path.rsplit('/', 1)[-1]: e for e in BigArchive(archive).entries}
        for bank in BOARD_BANKS:
            entry = wanted.get(bank)
            if entry is None:
                raise RuntimeError(f'Missing board sound bank: {bank}')
            if entry.compression:
                raise RuntimeError(f'Unexpected compressed sound bank: {bank}')
            raw.seek(entry.offset)
            source = work / bank
            source.write_bytes(raw.read(entry.stored_size))
            name = Path(bank).stem
            (output / name).mkdir(exist_ok=True)
            count = stream_count(vgmstream, source, env)

            def convert(index):
                wav = work / f'{name}-{index}.wav'
                decode(vgmstream, source, index, wav, env)
                encode(ffmpeg, wav, output / name / f'{index}.ogg', env, quality=5)
                wav.unlink()

            with ThreadPoolExecutor(workers()) as pool:
                list(pool.map(convert, range(1, count + 1)))
            report(f'Board sounds: {name} ({count} clips)')
    missing = [c for sound in BOARD_MAP.values() for c in sound['clips'] if not (output / c).is_file()]
    if missing:
        raise RuntimeError(f'Board clips missing: {missing[:5]}')
    target = output / 'board.json'
    # Keep a player's edited mapping across setup refreshes.
    if not target.is_file():
        target.write_text(json.dumps(BOARD_MAP, indent=2), encoding='utf-8')
    return output


def prepare_rolling(game_root, installation, vgmstream, ffmpeg, report=print, env=None):
    """The retail rolling layers and wheel spins, outside audiofiles.big.

    grains.big: per-surface rolling recordings ("<surface>_soft/hard.grain",
    low and high speed), each a seek table then an EA SNR stream at 0x90.
    wheels.big: free-spinning wheels winding down after a jump and in a
    manual (Whls_spins_Jump_1 / Whls_spins_Man_1 .snr)."""
    output = Path(installation) / 'assets/private/audio/board'
    audio = Path(game_root) / 'data/audio'
    count = 0
    with tempfile.TemporaryDirectory() as temp:
        work = Path(temp)
        jobs = []
        for bank, directory in (('grains.big', 'Grains'), ('wheels.big', 'Wheel_Spins')):
            archive = audio / bank
            if not archive.is_file():
                raise RuntimeError(f'Missing sound bank: {bank}')
            (output / directory).mkdir(parents=True, exist_ok=True)
            with open(archive, 'rb') as raw:
                for entry in BigArchive(archive).entries:
                    name = entry.path.rsplit('/', 1)[-1]
                    raw.seek(entry.offset)
                    data = raw.read(entry.stored_size)
                    if name.endswith('.grain'):
                        # The first word is the stream's offset past the seek table.
                        stem, stream = Path(name).stem, data[int.from_bytes(data[:4], 'big'):]
                    elif name.endswith('.snr'):
                        stem = {'Whls_spins_Jump_1': 'jump', 'Whls_spins_Man_1': 'manual'}.get(Path(name).stem, Path(name).stem)
                        stream = data
                    else:
                        continue
                    source = work / f'{directory}-{stem}.snr'
                    source.write_bytes(stream)
                    jobs.append((source, output / directory / f'{stem}.ogg'))

        def convert(job):
            source, target = job
            wav = source.with_suffix('.wav')
            subprocess.run([str(vgmstream), '-i', '-o', str(wav), str(source)], check=True,
                           stdout=subprocess.DEVNULL, env=env)
            encode(ffmpeg, wav, target, env, quality=5)

        with ThreadPoolExecutor(workers()) as pool:
            list(pool.map(convert, jobs))
        count = len(jobs)
    # Fresh loops: tools/seamless_loops.py must process them again.
    (output / '.seamless.json').unlink(missing_ok=True)
    report(f'Rolling surfaces and wheel spins ({count} clips)')
    return output


def prepare_body(game_root, installation, ffmpeg, report=print, env=None):
    """Body impact layers into board/<dir> (BODY_SOURCES); the game falls back
    to BODY_MAP's layout when board.json predates them."""
    from tools.ea_bank import samples, xma2_riff
    output = Path(installation) / 'assets/private/audio/board'
    archive = Path(game_root) / 'data/audio/audiofiles.big'
    entries = {e.path.rsplit('/', 1)[-1]: e for e in BigArchive(archive).entries}
    total = 0
    with tempfile.TemporaryDirectory() as temp, open(archive, 'rb') as raw:
        for bank_name, directory in BODY_SOURCES.items():
            entry = entries.get(bank_name)
            if entry is None or entry.compression:
                raise RuntimeError(f'Missing sound bank: {bank_name}')
            raw.seek(entry.offset)
            found = list(samples(raw.read(entry.stored_size)))
            wanted = sorted({int(Path(c).stem) for sound in BODY_MAP.values() for c in sound['clips']
                             if c.startswith(directory + '/')})
            if len(found) < wanted[-1]:
                raise RuntimeError(f'{bank_name}: only {len(found)} samples')
            (output / directory).mkdir(parents=True, exist_ok=True)

            def convert(number):
                _, rate, count, packets = found[number - 1]
                riff = Path(temp) / f'{directory}-{number}.wav'
                riff.write_bytes(xma2_riff(packets, rate, count))
                encode(ffmpeg, riff, output / directory / f'{number}.ogg', env, quality=5)

            with ThreadPoolExecutor(workers()) as pool:
                list(pool.map(convert, wanted))
            total += len(wanted)
    report(f'Body impact sounds ({total} clips)')
    return output


def playlist_gains(installation):
    """Per-song gain from the ipod_playlist record in the exported VLT database."""
    for database in sorted(Path(installation).glob('assets/private/customisation/sets/*/database/collections.json')):
        records = json.loads(database.read_text(encoding='utf-8'))['collections']
        record = next((r for r in records if r.get('key') == 'ipod_playlist'), None)
        if record:
            import struct
            items = record['fields']['ipod_playlist']['array']['items']
            return [(item[:8], struct.unpack('>f', bytes.fromhex(item[8:16]))[0]) for item in items]
    return None


def prepare_music(game_root, installation, vgmstream, ffmpeg, report=print, env=None):
    output = Path(installation) / 'assets/private/audio/music'
    output.mkdir(parents=True, exist_ok=True)
    source = Path(game_root) / 'data/audio/music/ipod.mpf'
    count = stream_count(vgmstream, source, env)
    if count % SEGMENTS_PER_SONG:
        raise RuntimeError(f'Unexpected iPod segment count {count}')
    songs = count // SEGMENTS_PER_SONG
    gains = playlist_gains(installation) or [(None, 1.0)] * songs
    if len(gains) != songs:
        gains = [(None, 1.0)] * songs
    with tempfile.TemporaryDirectory() as temp:
        work = Path(temp)

        def convert(song):
            first = song * SEGMENTS_PER_SONG + 1
            parts = []
            for index in range(first, first + SEGMENTS_PER_SONG):
                wav = work / f'{index}.wav'
                decode(vgmstream, source, index, wav, env)
                parts.append(wav)
            listing = work / f'song-{song}.txt'
            listing.write_text(''.join(f"file '{p}'\n" for p in parts), encoding='utf-8')
            encode(ffmpeg, None, output / f'{song + 1:02d}.ogg', env, concat_list=listing)
            for p in parts:
                p.unlink()
            listing.unlink()

        done = 0
        with ThreadPoolExecutor(workers()) as pool:
            for _ in pool.map(convert, range(songs)):
                done += 1
                report(f'Music: {done}/{songs} songs')
    playlist = [{'file': f'{i + 1:02d}.ogg', 'title': f'Track {i + 1}', 'id': gains[i][0],
                 'volume': round(gains[i][1], 3)} for i in range(songs)]
    (output / 'playlist.json').write_text(json.dumps(playlist, indent=2), encoding='utf-8')
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--game-root', type=Path, required=True)
    parser.add_argument('--installation', type=Path, required=True)
    parser.add_argument('--vgmstream', default='vgmstream-cli')
    parser.add_argument('--ffmpeg', default='ffmpeg')
    parser.add_argument('--skip-music', action='store_true')
    args = parser.parse_args()
    for tool in (args.vgmstream, args.ffmpeg):
        if not shutil.which(tool):
            raise SystemExit(f'Missing tool: {tool}')
    report = lambda text: print(text, flush=True)
    print(f'Board sounds ready: {prepare_board(args.game_root, args.installation, args.vgmstream, args.ffmpeg, report)}')
    if not args.skip_music:
        print(f'Music ready: {prepare_music(args.game_root, args.installation, args.vgmstream, args.ffmpeg, report)}')


if __name__ == '__main__':
    main()
