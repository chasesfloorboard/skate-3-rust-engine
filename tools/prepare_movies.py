"""The disc's FMV movies (data/movies/*.vp6: EA logo, the story intro, Coach
Frank, park-creation intros, film ender...), for the game's movie player.

Each movie is copied as is (the game decodes EA VP6 with ffmpeg at run time)
and its English soundtrack (interleaved EA SCHl 5.1 audio, which ffmpeg does
not read; vgmstream does) mixed to stereo Ogg Vorbis beside it.

Usage:
  python tools/prepare_movies.py GAME_ROOT INSTALLATION_DIR VGMSTREAM
"""
import argparse
import shutil
import subprocess
import tempfile
from pathlib import Path


def prepare_movies(game_root, installation, ffmpeg, vgmstream, report=print, env=None):
    source = Path(game_root) / 'data/movies'
    output = Path(installation) / 'assets/private/movies'
    output.mkdir(parents=True, exist_ok=True)
    count = 0
    for movie in sorted(source.glob('*_english_ntsc.vp6')):
        name = movie.name[:-len('_english_ntsc.vp6')]
        video, audio = output / f'{name}.vp6', output / f'{name}.ogg'
        if not video.is_file() or video.stat().st_size != movie.stat().st_size:
            shutil.copyfile(movie, video)
        if not audio.is_file():
            with tempfile.TemporaryDirectory() as temp:
                wav = Path(temp) / 'audio.wav'
                # Stream 1 is English (the SHEN header comes first).
                decoded = subprocess.run([str(vgmstream), '-i', '-s', '1', '-o', str(wav), str(movie)],
                                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env).returncode == 0
                if decoded and wav.is_file():
                    subprocess.run([str(ffmpeg), '-v', 'error', '-y', '-i', str(wav), '-ac', '2', '-c:a', 'libvorbis',
                                    '-q:a', '5', str(audio)], check=True, env=env)
        count += 1
    report(f'Movies ({count})')
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('game_root', type=Path)
    parser.add_argument('installation', type=Path)
    parser.add_argument('vgmstream', type=Path)
    args = parser.parse_args()
    prepare_movies(args.game_root, args.installation, shutil.which('ffmpeg') or 'ffmpeg', args.vgmstream)


if __name__ == '__main__':
    main()
