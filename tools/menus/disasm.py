"""Readable listing of a retail APT screen: timeline, placements and scripts.

  python tools/menus/disasm.py fedata/data/fe/source/screens/main/core_menu.apt [--grep NAME]

Opcode names follow SWF's ActionScript 1/2 set plus EA's APT extensions
(0x59-0x5D, 0x70-0x77, 0xA1-0xB9), matching crates/skate-game/src/apt_vm.rs.
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from vendor.skate3_ui.apt import inspect_apt  # noqa: E402
from menus.apt_actions import MenuActions  # noqa: E402

NAMES = {
    0x00: 'end', 0x04: 'nextFrame', 0x05: 'prevFrame', 0x06: 'play', 0x07: 'stop', 0x0A: 'add',
    0x0B: 'subtract', 0x0C: 'multiply', 0x0D: 'divide', 0x0E: 'equals', 0x0F: 'less', 0x10: 'and',
    0x11: 'or', 0x12: 'not', 0x17: 'pop', 0x18: 'toInteger', 0x1C: 'getVariable', 0x1D: 'setVariable',
    0x20: 'setTarget2', 0x21: 'stringAdd', 0x22: 'getProperty', 0x23: 'setProperty',
    0x24: 'cloneSprite', 0x25: 'removeSprite', 0x26: 'trace', 0x27: 'startDrag', 0x28: 'endDrag',
    0x30: 'random', 0x34: 'getTime', 0x3A: 'delete', 0x3B: 'delete2', 0x3C: 'defineLocal',
    0x3D: 'callFunction', 0x3E: 'return', 0x3F: 'modulo', 0x40: 'newObject', 0x41: 'defineLocal2',
    0x42: 'initArray', 0x43: 'initObject', 0x44: 'typeOf', 0x45: 'targetPath', 0x46: 'enumerate',
    0x47: 'add2', 0x48: 'less2', 0x49: 'equals2', 0x4A: 'toNumber', 0x4B: 'toString', 0x4C: 'pushDuplicate',
    0x4D: 'stackSwap', 0x4E: 'getMember', 0x4F: 'setMember', 0x50: 'increment', 0x51: 'decrement',
    0x52: 'callMethod', 0x53: 'newMethod', 0x54: 'instanceOf', 0x55: 'enumerate2',
    0x59: 'ea.pushZero', 0x5A: 'ea.pushOne', 0x5B: 'ea.callFunctionPop', 0x5C: 'ea.callFunction',
    0x5D: 'ea.callMethodPop', 0x60: 'bitAnd', 0x61: 'bitOr', 0x62: 'bitXor', 0x63: 'bitLShift',
    0x64: 'bitRShift', 0x65: 'bitURShift', 0x66: 'strictEquals', 0x67: 'greater', 0x69: 'extends',
    0x70: 'ea.pushThis', 0x71: 'ea.pushGlobal', 0x72: 'ea.zeroVar', 0x73: 'ea.pushTrue',
    0x74: 'ea.pushFalse', 0x75: 'ea.pushNull', 0x76: 'ea.pushUndefined', 0x77: 'ea.pushThisVar',
    0x81: 'gotoFrame', 0x83: 'getURL', 0x87: 'storeRegister', 0x88: 'constantPool', 0x8B: 'setTarget',
    0x8C: 'gotoLabel', 0x8E: 'defineFunction2', 0x94: 'with', 0x96: 'push', 0x99: 'jump',
    0x9A: 'getURL2', 0x9B: 'defineFunction', 0x9D: 'if', 0x9F: 'gotoFrame2',
    0xA1: 'ea.pushString', 0xA2: 'ea.pushConstant', 0xA3: 'ea.pushConstantWord',
    0xA4: 'ea.getStringVar', 0xA5: 'ea.getStringMember', 0xA6: 'ea.setStringVar',
    0xA7: 'ea.setStringMember', 0xAE: 'ea.pushValueOfVar', 0xAF: 'ea.getNamedMember',
    0xB0: 'ea.callNamedFuncPop', 0xB1: 'ea.callNamedFunc', 0xB2: 'ea.callNamedMethodPop',
    0xB3: 'ea.callNamedMethod', 0xB4: 'ea.pushFloat', 0xB5: 'ea.pushByte', 0xB6: 'ea.pushShort',
    0xB7: 'ea.pushLong', 0xB8: 'ea.branchIfFalse', 0xB9: 'ea.pushRegister',
}
CONSTANT_OPS = {0xA2, 0xA3, 0xAE, 0xAF, 0xB0, 0xB1, 0xB2, 0xB3}


def show(value):
    if isinstance(value, dict):
        return repr(value['value']) if value.get('kind') in (1, 6, 7) else f"<{value.get('kind')}>"
    return repr(value)


def member_name(rows, index, pool):
    """Name an anonymous function by the member it is stored into: AS2 pushes
    the owner and key, defines the function, then setMember."""
    if index + 1 < len(rows) and rows[index + 1]['opcode'] in (0x4F, 0x87, 0xA7, 0x3C, 0x1D):
        if rows[index + 1]['opcode'] == 0xA7:
            return '.' + rows[index + 1]['operand']
        before = rows[index - 1] if index else None
        if before and before['opcode'] in (0xA1, 0xA2, 0xA3) :
            value = before['operand'] if before['opcode'] == 0xA1 else (pool[before['operand']] if before['operand'] < len(pool) else None)
            if value is not None:
                return '.' + str(value)
        if before and before['opcode'] == 0x96 and before['values']:
            return '.' + str(before['values'][-1]['value'])
    return '<anonymous>'


def listing(rows, pool, indent='    '):
    lines = []
    for index, r in enumerate(rows):
        op = r['opcode']
        text = f"{indent}{r['offset']:05x}  {NAMES.get(op, f'op_{op:02x}')}"
        if op == 0x88:
            pool = [v['value'] for v in r['values']]
            text += f' [{len(pool)}]'
        elif 'values' in r:
            text += ' ' + ', '.join(show(v) for v in r['values'])
        elif op in CONSTANT_OPS and isinstance(r.get('operand'), int):
            k = r['operand']
            text += f' #{k}' + (f' {pool[k]!r}' if k < len(pool) else '')
        elif op == 0x83:
            text += f" {r['operand']!r} -> {r['target_name']!r}"
        elif 'operand' in r:
            text += f" {r['operand']!r}"
        if 'target' in r:
            text += f" -> {r['target']:05x}"
        lines.append(text)
        if 'body' in r:
            params = ', '.join(f"r{p['register']}:{p['name']}" if p['register'] else p['name'] for p in r['parameters'])
            lines[-1] += f" {r['name'] or member_name(rows, index, pool)}({params}) flags={r.get('flags', 0):#x}"
            lines += listing(r['body'], list(pool), indent + '    ')
    return lines


def disassemble(apt: Path):
    data = inspect_apt(apt, apt.with_suffix('.const'))
    actions = MenuActions(apt.read_bytes(), apt.with_suffix('.const').read_bytes())
    out = [f'# {apt.name}', 'imports: ' + ', '.join(f"{i.get('movie')}:{i.get('name')}" for i in data['imports']),
           'exports: ' + ', '.join(f"{e.get('name')}={e.get('character')}" for e in data['exports'])]
    owners = [('root', data['root'])] + [(f"character {c['id']} ({c['type_name']})", c) for c in data['characters']]
    for label, owner in owners:
        for number, frame in enumerate(owner.get('frames', [])):
            for control in frame['controls']:
                kind = control['type_name']
                head = f"{label} frame {number}: {kind}"
                if kind in ('place_object', 'place_object2', 'place_object3') or 'character_id' in control:
                    head += f" depth={control.get('depth')} char={control.get('character_id')} name={control.get('name')!r}"
                elif 'label' in control:
                    head += f" {control['label']!r}"
                out.append(head)
                offset = control.get('actions_offset')
                if offset and kind in ('do_action', 'do_init_action'):
                    out += listing(actions.stream(offset), [])
    return '\n'.join(out)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('apt', type=Path)
    p.add_argument('--grep', help='Only lines containing this text')
    args = p.parse_args()
    text = disassemble(args.apt)
    if args.grep:
        text = '\n'.join(line for line in text.splitlines() if args.grep in line)
    print(text)
