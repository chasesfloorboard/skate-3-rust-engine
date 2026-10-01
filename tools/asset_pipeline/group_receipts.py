"""Validate prepared groups without treating user files as extractor output."""
import json
from .setup_state import receipt, valid_receipt

ROOTS = {
    'core': ('assets/private/stock/data/anim', 'assets/private/stock/data/state',
             'assets/private/stock/data/config', 'assets/private/stock/data/cacrecipes',
             'assets/private/stock/skater-collections.json', 'assets/private/stock/physics-skeletons.json'),
    'hud': ('assets/private/hud', 'assets/private/session-marker',
            'assets/private/hud-availability.json', 'assets/private/session-marker-availability.json'),
    'character': ('assets/private/game.json', 'assets/private/skater.glb', 'assets/private/default_skater',
                  'assets/private/native-character', 'assets/private/character-lighting.json'),
    'environment': ('assets/private/native-skies', 'assets/private/native-backdrops',
                    'assets/private/render-parameters.json', 'assets/private/exposure.json',
                    'assets/private/exposure-profiles.json', 'assets/private/teleports.json', 'assets/private/environment-status'),
    'maps': ('assets/private/native-props', 'maps.json', 'assets/private/map-status'),
    'skate2': ('assets/private/skate2', 'assets/private/custom-locations', 'assets/private/audio/music-skate2',
               'assets/private/stock/skater-collections-skate2.json'),
}


def skate2_owned(root, path):
    """Skate 2 outputs written into folders other groups own: its physics in
    stock/ and its city's sky and scenery copies (keys start with S2)."""
    private = root/'assets/private'
    return (path.name == 'skater-collections-skate2.json'
            or (path.name.startswith('S2') and path.parent in (private/'native-skies', private/'native-backdrops')))


def files(root, group):
    result = []
    for name in ROOTS[group]:
        path = root/name
        result.extend(path.rglob('*') if path.is_dir() else [path])
    if group == 'core':
        stock = root/'assets/private/stock'
        result.extend(p for p in stock.rglob('*')
                      if not p.is_relative_to(stock/'data/content/createacharacter'))
    if group == 'skate2':
        private = root/'assets/private'
        result.extend(p for folder in ('native-skies', 'native-backdrops') for p in (private/folder).glob('S2*'))
    else:
        result = [p for p in result if not skate2_owned(root, p)]
    if group == 'maps':
        for item in json.loads((root/'maps.json').read_text()):
            path = (root/item['path']).resolve()
            if not path.is_relative_to((root/'maps').resolve()):
                raise ValueError('Invalid prepared map path')
            result.append(path)
    return result


def record(root, group):
    return receipt(root, files(root, group))


def damaged(root, marker, exclude=()):
    saved = marker.get('outputs', {})
    result = set()
    for group, roots in ROOTS.items():
        if group in exclude:continue
        if group in saved:
            if not valid_receipt(root, saved[group]):result.add(group)
        else:
            # Legacy packages have no receipts: require their declared roots
            # and verify the map checksums they already recorded. The runtime
            # validator also checks legacy core inputs before publication.
            if any(not (root/name).exists() for name in roots if not name.endswith(('-availability.json', '-status'))):result.add(group)
            if group == 'hud' and group not in result:
                from tools.install_prepared_hud import runtime_files
                try:
                    runtime_files(root/'assets/private/hud')
                    runtime_files(root/'assets/private/session-marker', marker=True)
                except (OSError, ValueError, KeyError, TypeError):result.add(group)
            if group == 'maps' and group not in result:
                try:
                    maps = json.loads((root/'maps.json').read_text())
                    checks = {m['path']: {'size': (root/m['path']).stat().st_size,
                                         'sha256': m['sha256']} for m in maps}
                    if not valid_receipt(root, checks):result.add(group)
                except (OSError, ValueError, KeyError, TypeError):result.add(group)
    if 'core' in result:result.update(ROOTS)
    return result
