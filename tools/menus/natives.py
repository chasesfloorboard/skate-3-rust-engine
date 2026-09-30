"""List the native API the retail menus expect from the game engine.

  python tools/menus/natives.py <menus dir from prepare_menus.py>

Finds each `_global.X` that no script assigns (engine-injected objects) and the
methods called on them, with argument counts where the push is visible.
"""
import json
import sys
from collections import defaultdict
from pathlib import Path

NATIVES = {'ScreenManager', 'Audio', 'LetterBox', 'ReplayEditor', 'Mission', 'HUDComponents', 'tos',
           'Game', 'Tricks', 'Profile'}


def name_of(r, pool):
    if r['opcode'] in (0xA1, 0xA4, 0xA5):
        return r.get('operand')
    if r['opcode'] in (0xA2, 0xA3, 0xAE, 0xAF) and isinstance(r.get('operand'), int) and r['operand'] < len(pool):
        return pool[r['operand']]
    if r['opcode'] == 0x96 and r['values'] and r['values'][-1]['kind'] == 1:
        return r['values'][-1]['value']


def scan(rows, pool, calls, movie):
    for i, r in enumerate(rows):
        if r['opcode'] == 0x88:
            pool = [v['value'] for v in r['values']]
        if 'body' in r:
            scan(r['body'], list(pool), calls, movie)
        if r['opcode'] in (0x52, 0x5D) and i >= 2:
            method, owner = name_of(rows[i - 1], pool), name_of(rows[i - 2], pool)
            if owner in NATIVES and method:
                argc = None
                for k in range(i - 3, max(-1, i - 5), -1):
                    op = rows[k]['opcode']
                    if op in (0xB5, 0x59, 0x5A):
                        argc = rows[k]['operand'] if op == 0xB5 else int(op == 0x5A)
                        break
                calls[owner][method].add((argc, movie))
        if r['opcode'] in (0xB2, 0xB3) and i >= 1:
            owner = name_of(rows[i - 1], pool)
            method = pool[r['operand']] if r['operand'] < len(pool) else None
            if owner in NATIVES and method:
                calls[owner][method].add((None, movie))


def main(root):
    calls = defaultdict(lambda: defaultdict(set))
    for f in sorted(Path(root).rglob('*.json')):
        if f.name == 'index.json':
            continue
        screen = json.loads(f.read_text())
        for rows in screen['actions'].values():
            scan(rows, [], calls, screen['movie'])
    for owner, methods in sorted(calls.items()):
        print(f'## {owner} ({len(methods)})')
        for method, uses in sorted(methods.items()):
            counts = sorted({str(a) for a, _ in uses if a is not None}) or ['?']
            movies = sorted({m.rsplit('/', 1)[-1].removesuffix('.swf') for _, m in uses})
            print(f"- {method}({'|'.join(counts)}) - {', '.join(movies[:6])}{' ...' if len(movies) > 6 else ''}")


if __name__ == '__main__':
    main(sys.argv[1])
