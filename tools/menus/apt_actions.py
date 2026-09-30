"""APT bytecode reader for the retail front-end screens.

Extends the HUD reader (vendor/skate3_ui/actions.py) with the one compound
opcode only the menus use. It lives here rather than in the vendored file
because that file is part of the HUD setup stage's version hash: editing it
would send every installation back through HUD conversion.

GetURL (0x83) in main.apt: two aligned string pointers, url then target, e.g.
getURL("source/screens/main3dhud.swf", "_level1") loads a screen into a level.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from vendor.skate3_ui.actions import Actions, BYTE, SHORT, WORD, STRING  # noqa: E402
from vendor.skate3_ui.binary import FormatError, align  # noqa: E402

GET_URL = 0x83


class MenuActions(Actions):
    def stream(self, offset, end=None, nesting=0):
        """Decode like Actions.stream, splitting around GetURL instructions."""
        r = self.apt
        end = len(r.data) if end is None else end
        result = []
        while offset < end:
            # Plain instructions up to a GetURL, the terminator or the end go
            # to the base reader, which refuses the compound opcode.
            cursor, terminated = offset, False
            while cursor < end and r.u8(cursor) != GET_URL:
                if r.u8(cursor) == 0:
                    cursor, terminated = cursor + 1, True
                    break
                cursor = self._next(cursor)
            if cursor > offset:
                result += super().stream(offset, cursor, nesting)
            if terminated or cursor >= end:
                break
            operand = align(cursor + 1, 4)
            url, target = r.cstring(r.u32be(operand)), r.cstring(r.u32be(operand + 4))
            result.append({'offset': cursor, 'opcode': GET_URL, 'operand': url,
                           'target_name': target, 'next': operand + 8})
            offset = operand + 8
        return result

    def _next(self, offset):
        """Offset after one plain instruction (same sizes as the base reader)."""
        r = self.apt
        op, nxt = r.u8(offset), offset + 1
        if op in BYTE:
            nxt += 1
        elif op in SHORT:
            nxt += 2
        elif op in {0x77, 0xB4, 0xB7}:
            nxt += 4
        elif op in WORD | STRING | {0x94}:
            nxt = align(nxt, 4) + 4
        elif op in {0x88, 0x96}:
            nxt = align(nxt, 4) + 8
        elif op in {0x8E, 0x9B}:
            nxt = align(nxt, 4)
            length = r.u32be(nxt + (16 if op == 0x8E else 12))
            nxt += (28 if op == 0x8E else 24) + length
        elif op == 0x8F:
            raise FormatError(f'APT compound opcode {op:#x} at {offset:#x} needs decoding')
        return nxt
