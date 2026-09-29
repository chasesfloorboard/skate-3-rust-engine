"""Add the kart customiser (Mario Kart 8 bodies and wheels built by
mk8_kart_parts.py) to the Mario Kart mod: "Kart body" and "Wheels" choices in
its settings, and main.lua passing the chosen parts to sdk.vehicle.spawn and
scaling tyre grip.

Usage:
  python tools/mk8_kart_mod.py ORIGINAL_MOD.zip PARTS_MOD_DIR OUT.zip
    ORIGINAL_MOD.zip  the unmodified Mario Kart mod package
    PARTS_MOD_DIR     folder holding parts/ (mk8_kart_parts.py output)
"""
import argparse
import json
import zipfile
from pathlib import Path

HEADER = "-- The original Lua/JSON example uses the user's separately supplied kart model.\n"


def lua(value):
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def rotation(axis, degrees):
    import numpy as np
    a = np.radians(degrees)
    c, s_ = np.cos(a), np.sin(a)
    x, y, z = np.asarray(axis, float) / np.linalg.norm(axis)
    return np.array([[c + x * x * (1 - c), x * y * (1 - c) - z * s_, x * z * (1 - c) + y * s_],
                     [y * x * (1 - c) + z * s_, c + y * y * (1 - c), y * z * (1 - c) - x * s_],
                     [z * x * (1 - c) - y * s_, z * y * (1 - c) + x * s_, c + z * z * (1 - c)]])


def turned_to(current, target):
    """Rotation matrix taking direction `current` onto `target`."""
    import numpy as np
    a, b = current / np.linalg.norm(current), target / np.linalg.norm(target)
    axis = np.cross(a, b)
    if np.linalg.norm(axis) < 1e-6:
        return np.eye(3)
    return rotation(axis, np.degrees(np.arccos(np.clip(a @ b, -1, 1))))


def straddle(rider):
    """straddle_* copies of the drive/steer clips for bikes and ATVs: thighs
    pitched down and splayed a little, shins down and back to the pegs. The
    clip frames hold model-space bone matrices (hips at the origin, +Z
    forward, +Y up, +X the rider's left)."""
    import numpy as np
    names = rider['bone_names']
    at = {n: i for i, n in enumerate(names)}
    for clip in ('drive', 'steer_left', 'steer_right'):
        if clip not in rider['clips'] or f'straddle_{clip}' in rider['clips']:
            continue
        source = rider['clips'][clip]
        frames = []
        for frame in source['frames']:
            m = [np.array(v, float).reshape(4, 4).T for v in frame]
            for side, out in (('RIGHT', -1.0), ('LEFT', 1.0)):
                chain = [at[f'{side}{b}'] for b in ('UPLEG', 'LEG', 'FOOT', 'TOEBASE') if f'{side}{b}' in at]
                if len(chain) < 3:
                    continue
                def turn(joints, pivot, r):
                    for j in joints:
                        m[j][:3, :3] = r @ m[j][:3, :3]
                        m[j][:3, 3] = pivot + r @ (m[j][:3, 3] - pivot)
                hip, knee, foot = (m[j][:3, 3].copy() for j in chain[:3])
                # Thigh 55 degrees below level, knees 8 degrees out.
                thigh = np.array([out * np.sin(np.radians(8)), -np.sin(np.radians(55)), np.cos(np.radians(55))])
                turn(chain, hip, turned_to(knee - hip, thigh))
                knee, foot = m[chain[1]][:3, 3].copy(), m[chain[2]][:3, 3].copy()
                # Shin down and slightly back to the footpeg.
                turn(chain[1:], knee, turned_to(foot - knee, np.array([0.0, -0.95, -0.3])))
            frames.append([list(x.T.reshape(-1)) for x in m])
        rider['clips'][f'straddle_{clip}'] = {'fps': source['fps'], 'frames': frames}
    return rider


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('original', type=Path)
    parser.add_argument('parts', type=Path)
    parser.add_argument('out', type=Path)
    args = parser.parse_args()
    listing = json.loads((args.parts / 'parts/parts.json').read_text())
    with zipfile.ZipFile(args.original) as source:
        files = {name: source.read(name) for name in source.namelist() if not name.endswith('/')}

    mod = json.loads(files['mod.json'])
    settings = mod['settings']
    settings.pop('driver', None)
    settings['body'] = {'type': 'choice', 'label': 'Kart body',
                        'description': 'Mario Kart 8 body (a parked kart respawns with it).',
                        'default': 'Original kart', 'choices': ['Original kart'] + [b['name'] for b in listing['bodies']]}
    most = min(64, max(len(b.get('styles', [b['file']])) for b in listing['bodies']))
    settings['style'] = {'type': 'choice', 'label': 'Kart style',
                         'description': 'Colour scheme of the Mario Kart 8 body. Bodies without that many styles use their first.',
                         'default': 'Style 1', 'choices': [f'Style {i + 1}' for i in range(most)]}
    settings['landing_assist'] = {'type': 'boolean', 'label': 'Landing assist',
                                  'description': 'In the air the kart turns to land wheels-down on the ramp or ground ahead, like a skateboard, instead of barrel rolling.',
                                  'default': True}
    tyre_names = [t['name'] for t in listing['tyres']]
    settings['wheels'] = {'type': 'choice', 'label': 'Wheels',
                          'description': 'Tyres for Mario Kart 8 bodies. Each grips differently: Slick most, Metal least.',
                          'default': 'Standard' if 'Standard' in tyre_names else tyre_names[0], 'choices': tyre_names}
    # Mod menu pictures (kart previews rendered by mk8_kart_parts.py): the
    # body in its chosen style, and the tyre.
    # "<body>" is the first style; the game falls back to it for style
    # numbers a body lacks, as chosen_parts() does.
    looks = {}
    for b in listing['bodies']:
        for i, preview in enumerate(b.get('previews', [])):
            looks[b['name'] + (f'|Style {i + 1}' if i else '')] = preview
    if listing.get('original_preview'):
        looks['Original kart'] = listing['original_preview']
    for key in ('body', 'style'):
        settings[key]['previews'] = looks
        settings[key]['preview_from'] = ['body', 'style']
    settings['wheels']['previews'] = {t['name']: t['preview'] for t in listing['tyres'] if t.get('preview')}
    mod['version'] = '1.8.0'
    files['mod.json'] = json.dumps(mod, separators=(',', ':')).encode()

    table = ['local PARTS={bodies={']
    table += [f' [{lua(b["name"])}]={{' + ','.join(lua(f) for f in b.get('styles', [b['file']]))
              + (f',layout={lua(b["layout"])}' if b.get('layout') else '')
              + (',seat={' + ','.join(f'{v:.3f}' for v in b['seat']) + '}' if b.get('seat') else '')
              + (',wheel_z={' + ','.join(f'{v:.3f}' for v in b['wheel_z']) + '}' if b.get('wheel_z') else '') + '},'
              for b in listing['bodies']]
    table += ['},tyres={']
    table += [f' [{lua(t["name"])}]={{file={lua(t["file"])},radius={t["radius"]:.4f},grip={t["grip"]}}},'
              for t in listing['tyres']]
    table += ['}}']
    script = files['main.lua'].decode()
    assert HEADER in script, 'unexpected main.lua'
    script = script.replace(HEADER, HEADER + "-- Mario Kart 8 parts (tools/mk8_kart_parts.py) chosen in the mod settings.\n"
                            + '\n'.join(table) + '\n' + '''local function chosen_parts()
 local s=sdk.settings
 local tyre=PARTS.tyres[s.wheels] or PARTS.tyres['Standard']
 local styles=PARTS.bodies[s.body]
 if not styles then return nil,nil end
 local n=tonumber(tostring(s.style or ''):match('%d+')) or 1
 return {body=styles[n] or styles[1],wheels=tyre.file,wheel_radius=tyre.radius,layout=styles.layout,seat=styles.seat,wheel_z=styles.wheel_z},tyre
end
local build=''
local function build_key() local s=sdk.settings;return tostring(s.body)..'|'..tostring(s.style)..'|'..tostring(s.wheels) end
''', 1)
    replacements = [
        (" local s=sdk.settings\n sdk.vehicle.tune('kart',{engine_volume=s.engine_volume,engine_force=s.engine,max_speed=s.speed,"
         "brake_impulse=s.brake,steering_angle=s.steering,tire_grip=s.tire_friction})",
         " local s=sdk.settings\n local _,tyre=chosen_parts()\n"
         " -- Each tyre's grip scales the friction setting (1.3 is the dry baseline).\n"
         " local grip=math.min(2,s.tire_friction*(tyre and tyre.grip/1.3 or 1))\n"
         " sdk.vehicle.tune('kart',{engine_volume=s.engine_volume,engine_force=s.engine,max_speed=s.speed,"
         "brake_impulse=s.brake,steering_angle=s.steering,tire_grip=grip,landing_assist=s.landing_assist and 1 or 0})"),
        (" local p,h=nearby();sdk.vehicle.spawn('kart','vehicle.json',p,h);tune()",
         " local p,h=nearby();sdk.vehicle.spawn('kart','vehicle.json',p,h,(chosen_parts()));build=build_key();tune()"),
        (" on_settings=function() local car=sdk.vehicle.read('kart');if car then tune() end;hud(car) end,",
         " on_settings=function()\n  local car=sdk.vehicle.read('kart')\n"
         "  -- A new body or wheels respawns the parked kart where it stands.\n"
         "  if car and not car.occupied and build_key()~=build then\n"
         "   sdk.vehicle.remove('kart');sdk.vehicle.spawn('kart','vehicle.json',{car.position[1],car.position[2]+0.5,car.position[3]},"
         "car.heading,(chosen_parts()));build=build_key()\n  end\n  if car then tune() end;hud(car)\n end,"),
    ]
    replacements.append(("'Slow below 3 m/s to exit | ", "'Jump out at speed to bail | "))
    for old, new in replacements:
        assert old in script, old
        script = script.replace(old, new)
    files['main.lua'] = script.encode()
    files['rider.json'] = json.dumps(straddle(json.loads(files['rider.json'])), separators=(',', ':')).encode()
    for path in sorted((args.parts / 'parts').rglob('*')):
        if path.is_file():
            files[path.relative_to(args.parts).as_posix()] = path.read_bytes()
    with zipfile.ZipFile(args.out, 'w', zipfile.ZIP_DEFLATED) as out:
        for name in sorted(files):
            out.writestr(name, files[name])
    print(f"{args.out}: {len(listing['bodies'])} bodies, {len(listing['tyres'])} tyres")


if __name__ == '__main__':
    main()
