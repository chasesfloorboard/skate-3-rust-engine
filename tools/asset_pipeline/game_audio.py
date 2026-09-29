"""Optional extras from the disc: ambience, board sounds, soundtrack, the
retail menu skin and the challenge map.

Kept out of the customiser fingerprint: changes here never invalidate characters."""


def prepare(game, stage, report):
    # Front-end assets need no external tools.
    from tools.prepare_menu_skin import prepare as prepare_menu_skin
    from tools.prepare_map_ui import prepare as prepare_map_ui
    for label, step in (('Menu skin', prepare_menu_skin), ('Challenge map', prepare_map_ui)):
        try:
            step(game, stage, report)
        except Exception as error:
            report(f'{label} unavailable: {error}')
    from tools.asset_pipeline.install import bundled_tool, system_env
    vgmstream, ffmpeg = bundled_tool('vgmstream-cli'), bundled_tool('ffmpeg')
    if vgmstream is None or ffmpeg is None:
        report('Skipping game audio: ' + ('ffmpeg' if vgmstream else 'vgmstream-cli') + ' is not installed')
        return
    from tools.prepare_ambience import prepare as prepare_ambience
    from tools.prepare_audio import prepare_board, prepare_body, prepare_music
    # Each part is independent; the game runs without any of them.
    for label, step in (('Ambience', prepare_ambience), ('Board sounds', prepare_board), ('Music', prepare_music)):
        try:
            step(game, stage, vgmstream, ffmpeg, report, system_env())
        except Exception as error:
            report(f'{label} unavailable: {error}')
    try:
        prepare_body(game, stage, ffmpeg, report, system_env())
    except Exception as error:
        report(f'Body impact sounds unavailable: {error}')
    try:
        from tools.prepare_audio import prepare_rolling
        prepare_rolling(game, stage, vgmstream, ffmpeg, report, system_env())
    except Exception as error:
        report(f'Rolling surface sounds unavailable: {error}')
    try:
        from tools.prepare_movies import prepare_movies
        prepare_movies(game, stage, ffmpeg, vgmstream, report, system_env())
    except Exception as error:
        report(f'Movies unavailable: {error}')
    try:
        from tools.seamless_loops import process as seamless_loops
        seamless_loops(stage, ffmpeg, system_env())
    except Exception as error:
        report(f'Seamless board loops unavailable: {error}')
