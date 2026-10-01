"""Setup's optional "skate2" group: everything the Skate 2 edition uses.

From a Skate 2 ISO or extracted disc (default.xex beside data/), plus any
Skate 2 DLC packages (the downloaded .zip files, bare LIVE packages or a
folder of them):
  - the city and DLC parks as custom locations, with their spots, distant
    scenery and Skate 2 physics (tools/skate2/import_locations.py)
  - the disc's movies (assets/private/skate2/movies)
  - the soundtrack (assets/private/audio/music-skate2)
  - pros and story characters (assets/private/skate2/native-roster)
Every part is optional content: a failure is reported and setup continues.
assets/private/skate2/status.json always records the outcome, so a copy set
up without Skate 2 is complete too (and setup offers it again later).
"""
import json
from pathlib import Path

STATUS = 'assets/private/skate2/status.json'


def disc_root(source, work, base, report, log):
    """An extracted Skate 2 disc directory for an ISO, default.xex or folder."""
    from .install import xiso_extractor, run
    source = Path(source).resolve()
    if source.is_file() and source.suffix.lower() == '.iso':
        root = work / 'skate2-disc'
        report('Extracting your Skate 2 ISO')
        run([xiso_extractor(base, report), '-x', source, '-d', root], log, report)
    elif source.is_file():
        root = source.parent
    else:
        root = source
    # extract-xiso writes into a folder named after the ISO.
    if not (root / 'data/content/worldbam.big').is_file():
        nested = [p for p in root.iterdir() if (p / 'data/content/worldbam.big').is_file()] if root.is_dir() else []
        if not nested:
            raise RuntimeError('This is not a Skate 2 disc: missing data/content/worldbam.big')
        root = nested[0]
    return root


def write_status(stage, value):
    from .setup_state import atomic_json
    path = stage / STATUS
    path.parent.mkdir(parents=True, exist_ok=True)
    atomic_json(path, {'version': 1, **value})


def prepare(source, dlc, stage, game_exe, work, base, report, log):
    """Prepare the group into `stage`. source None: Skate 2 not provided."""
    if source is None:
        write_status(stage, {'status': 'not-provided'})
        report('Skate 2: not provided (choose your Skate 2 disc in setup to add it)')
        return
    from .install import run, task, bundled_tool, system_env, TOOLS
    from .optional_content import CONTENT_ERRORS, note
    private = stage / 'assets/private'
    work = Path(work) / 'skate2'
    work.mkdir(parents=True, exist_ok=True)
    root = disc_root(source, work, base, report, log)
    parts = {}

    def step(label, availability, action):
        try:
            action()
            (private / availability).unlink(missing_ok=True)
            parts[label] = 'ready'
        except CONTENT_ERRORS as error:
            note(private / availability, label, error, report=report)
            parts[label] = 'unavailable'

    def locations():
        report('Skate 2: converting the city and DLC parks (this is the longest step)')
        args = ['--skate2', root, '--installation', stage, '--work', work / 'locations', '--game', game_exe]
        if dlc:
            args += ['--dlc', dlc]
        run(task(TOOLS / 'skate2/import_locations.py', *args), log, report)
        if not any((private / 'custom-locations').glob('*/location.json')):
            raise RuntimeError('No Skate 2 location was converted; see ' + str(log.name))

    def media(kind):
        vgmstream, ffmpeg = bundled_tool('vgmstream-cli'), bundled_tool('ffmpeg')
        if vgmstream is None or ffmpeg is None:
            raise RuntimeError(('ffmpeg' if vgmstream else 'vgmstream-cli') + ' is not installed')
        if kind == 'movies':
            from tools.prepare_movies import prepare_movies
            prepare_movies(root, stage, ffmpeg, vgmstream, report, system_env(), output_dir='skate2/movies')
        else:
            from tools.prepare_audio import prepare_music
            prepare_music(root, stage, vgmstream, ffmpeg, report, system_env(), skate2=True)

    def roster():
        from tools.skate2.roster import prepare as prepare_roster
        result = prepare_roster(root, stage, work / 'roster', report)
        if not any(r['status'] == 'ready' for r in result):
            raise RuntimeError('No Skate 2 character could be converted')

    step('Skate 2 maps and physics', 'skate2/locations-availability.json', locations)
    step('Skate 2 movies', 'skate2/movies-availability.json', lambda: media('movies'))
    step('Skate 2 soundtrack', 'skate2/music-availability.json', lambda: media('music'))
    step('Skate 2 characters', 'skate2/characters-availability.json', roster)
    write_status(stage, {'status': 'ready', 'source': str(Path(source).resolve()),
                         'dlc': str(Path(dlc).resolve()) if dlc else None, 'parts': parts})


def status(root):
    """The recorded outcome for an installation, or None before this group existed."""
    try:
        return json.loads((Path(root) / STATUS).read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return None
