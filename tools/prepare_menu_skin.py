"""Extract the retail menu look (Futura Heavy bitmap font, selection chevron and
slider pill) from the disc front end for the in-game menu.

Output: <installation>/assets/private/ui/menu/{futuraheavy.png, font.json,
arrow.png, pill.png, icons/*.png}. font.json maps characters to atlas rectangles (left, top, width, height),
bearings and advances in the font's native 28 px size.

Usage:
  python tools/prepare_menu_skin.py --game-root DISC --installation INSTALL_DIR
"""
import argparse
import json
import shutil
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent / 'vendor'))
from skate3_ui.bitmap_font import parse_bitmap_font  # noqa: E402
from skate3_ui.project import extract_project  # noqa: E402

FONT = 'futuraheavy'
# Soft halo drawn behind highlighted text, as on the retail menus.
GLOW_FONT = 'futuraglow'
PREFIXES = (f'data/fe/fonts/{FONT}', f'data/fe/fonts/{GLOW_FONT}', 'data/fe/source/controls/menu_part_arrow',
            'data/fe/source/controls/highlight_slider', 'data/fe/source/screens/main/core_menu')
# Pause-menu icons by core_menu texture index (64x64 blue tiles; tab icons
# have bright/selected and dim variants).
ICONS = {
    'challenge_map': 59, 'edit_skater': 23, 'call_skater': 1, 'district': 20, 'free_play': 24,
    'quit': 60, 'party_play': 32, 'mods': 21, 'day_night': 22, 'updates': 17, 'options': 8,
    'audio': 29, 'feed': 26, 'banner': 52,
    'tab_main': 50, 'tab_online': 51, 'tab_extras': 53, 'tab_options': 55,
    'tab_main_dim': 44, 'tab_online_dim': 45, 'tab_extras_dim': 46, 'tab_options_dim': 48,
}


def only(folder, pattern):
    matches = sorted(Path(folder).glob(pattern))
    if not matches:
        raise RuntimeError(f'Missing menu asset {folder}/{pattern}')
    return matches


def prepare(game_root, installation, report=print):
    output = Path(installation) / 'assets/private/ui/menu'
    with tempfile.TemporaryDirectory() as temp:
        cache = Path(temp) / 'ui'
        extract_project(Path(game_root), cache, prefixes=PREFIXES, decode_png=True, force=True)
        assets = cache / 'assets/data/fe'
        font = parse_bitmap_font(only(cache / 'raw/data/fe/fonts', f'{FONT}.bmpFont')[0])
        atlas = only(assets / f'fonts/{FONT}', '*.png')[0]
        glow = parse_bitmap_font(only(cache / 'raw/data/fe/fonts', f'{GLOW_FONT}.bmpFont')[0])
        glow_atlas = only(assets / f'fonts/{GLOW_FONT}', '*.png')[0]
        # menu_part_arrow holds a 32x16 glow and the 16x16 chevron; the pill is
        # highlight_slider's only texture.
        arrow = next(p for p in only(assets / 'source/controls/menu_part_arrow', '*.png')
                     if p.name.startswith('0001_'))
        pill = only(assets / 'source/controls/highlight_slider', '*.png')[0]
        (output / 'icons').mkdir(parents=True, exist_ok=True)
        core = assets / 'source/screens/main/core_menu'
        for name, index in ICONS.items():
            shutil.copyfile(only(core, f'{index:04d}_*.png')[0], output / 'icons' / f'{name}.png')
        output.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(atlas, output / f'{FONT}.png')
        shutil.copyfile(glow_atlas, output / f'{GLOW_FONT}.png')
        shutil.copyfile(arrow, output / 'arrow.png')
        shutil.copyfile(pill, output / 'pill.png')
    def table(font):
        glyphs = {g['glyph_index']: g for g in font['glyphs']}
        characters = {}
        for entry in font['characters']:
            g = glyphs.get(entry['glyph_index'])
            if g is None:
                continue
            # x/y are the pen origin; atlas_bounds is the actual glyph rectangle.
            left, top = g['atlas_bounds'][:2]
            characters[chr(entry['codepoint'])] = [left, top, g['width'], g['height'],
                                                    g['x_offset'], g['y_offset'], g['x_advance']]
        return characters
    metrics = dict(font['metrics'])
    characters = table(font)
    (output / 'font.json').write_text(json.dumps({
        'atlas': f'{FONT}.png', 'size': metrics.get('Size', 28),
        'baseline': metrics.get('Baseline', 44), 'line_height': metrics.get('LineHeight', 50),
        'characters': characters,
        'glow': {'atlas': f'{GLOW_FONT}.png', 'size': dict(glow['metrics']).get('Size', 28), 'characters': table(glow)},
    }), encoding='utf-8')
    report(f'Menu skin: {len(characters)} characters')
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--game-root', type=Path, required=True)
    parser.add_argument('--installation', type=Path, required=True)
    args = parser.parse_args()
    print(f'Menu skin ready: {prepare(args.game_root, args.installation)}')


if __name__ == '__main__':
    main()
