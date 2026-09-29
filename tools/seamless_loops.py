"""Make the board sound loops repeat without a click.

Several retail clips used as loops (rolling surfaces, grinds, wind, body
slides) fade to silence at one end or jump at the seam, which pops every time
the loop wraps. Each is trimmed of silent edges and its tail cross-faded into
its head, in place. A marker file lists the processed clips so running again
changes nothing.

Usage:
  python tools/seamless_loops.py INSTALLATION_DIR
"""
import argparse
import json
import subprocess
import tempfile
from pathlib import Path

import numpy as np

# Clips the engine plays only as loops (board_audio.rs LOOPS/SURFACE_LOOPS
# and board.json's loop entries).
LOOPS = [f'PatchBank_Rolling_Surfaces/{n}.ogg' for n in range(1, 8)] + [
    'Rolling_Rattles/1.ogg', 'GRINDS/1.ogg', 'GRINDS/2.ogg', 'sense_of_speed/1.ogg',
    'Bodyslide/1.ogg', 'Bodyslide/11.ogg'] + [
    f'Grains/{surface}_{speed}.ogg' for surface in ('asphalt_rough', 'asphalt_smooth', 'concrete_aggregate',
                                                   'concrete_rough', 'concrete_smooth', 'wood_ramp')
    for speed in ('soft', 'hard')] + ['Grains/metal_smooth_hard.ogg']
RATE = 44100
FADE = 0.08


def decode(path, ffmpeg='ffmpeg', env=None):
    raw = subprocess.run([str(ffmpeg), '-v', 'error', '-i', str(path), '-ac', '1', '-ar', str(RATE), '-f', 'f32le', '-'],
                         capture_output=True, check=True, env=env).stdout
    return np.frombuffer(raw, np.float32).copy()


def seamless(x):
    # Trim near-silent edges (below 5% of the clip's loudness, 10 ms windows).
    window = RATE // 100
    level = np.sqrt(np.convolve(x * x, np.ones(window) / window, 'same'))
    loud = np.nonzero(level > 0.05 * np.sqrt((x * x).mean()))[0]
    if len(loud):
        x = x[loud[0]:loud[-1] + 1]
    fade = min(int(FADE * RATE), len(x) // 4)
    if fade < 16:
        return x
    # Equal-power cross-fade of the tail into the head; the tail is dropped.
    t = np.linspace(0.0, np.pi / 2, fade)
    head = x[:fade] * np.sin(t) + x[-fade:] * np.cos(t)
    return np.concatenate([head, x[fade:-fade]]).astype(np.float32)


def process(installation, ffmpeg='ffmpeg', env=None):
    board = Path(installation) / 'assets/private/audio/board'
    marker = board / '.seamless.json'
    done = set(json.loads(marker.read_text())) if marker.is_file() else set()
    for clip in LOOPS:
        path = board / clip
        if clip in done or not path.is_file():
            continue
        loop = seamless(decode(path, ffmpeg, env))
        with tempfile.TemporaryDirectory() as temp:
            raw = Path(temp) / 'loop.f32'
            raw.write_bytes(loop.tobytes())
            out = Path(temp) / 'loop.ogg'
            subprocess.run([str(ffmpeg), '-v', 'error', '-y', '-f', 'f32le', '-ar', str(RATE), '-ac', '1', '-i', str(raw),
                            '-c:a', 'libvorbis', '-q:a', '5', str(out)], check=True, env=env)
            path.write_bytes(out.read_bytes())
        done.add(clip)
    marker.write_text(json.dumps(sorted(done), indent=1))
    return sorted(done)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('installation', type=Path)
    print(process(parser.parse_args().installation))


if __name__ == '__main__':
    main()
