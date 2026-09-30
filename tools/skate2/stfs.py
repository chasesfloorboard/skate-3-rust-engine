"""Minimal Xbox 360 STFS (LIVE/PIRS/CON) extractor: python stfs.py PACKAGE OUTDIR"""
import struct, sys
from pathlib import Path

class Stfs:
    def __init__(self, path):
        self.f = open(path, 'rb')
        h = self.f.read(0x1000)
        self.magic = h[:4]
        self.header_size = struct.unpack('>I', h[0x340:0x344])[0]
        self.name = h[0x411:0x411+0x80].decode('utf-16-be', 'ignore').rstrip('\0')
        self.block_sep = h[0x37B]
        self.ft_count = struct.unpack('<H', h[0x37C:0x37E])[0]
        self.ft_block = int.from_bytes(h[0x37E:0x381], 'little')
        self.base = (self.header_size + 0xFFF) & ~0xFFF
        if self.base == 0xB000: self.shift = 0
        else: self.shift = 0 if self.block_sep & 1 else 1
        self.spacing = [(0xAB, 0x718F, 0xFE7DA), (0xAC, 0x723A, 0xFD00B)][self.shift]

    def fix(self, b):
        r = (((b + 0xAA) // 0xAA) << self.shift) + b
        if b < 0xAA: return r
        r += ((b + 0x70E4) // 0x70E4) << self.shift
        return r if b < 0x70E4 else r + (1 << self.shift)

    def block(self, b):
        self.f.seek(self.base + self.fix(b) * 0x1000)
        return self.f.read(0x1000)

    def hash_entry_offset(self, b):
        record = b % 0xAA
        t = (b // 0xAA) * self.spacing[0]
        if b >= 0xAA:
            t += ((b // 0x70E4) + 1) << self.shift
            if b >= 0x70E4: t += 1 << self.shift
        return self.base + (t << 12) + record * 0x18

    def next_block(self, b):
        self.f.seek(self.hash_entry_offset(b) + 0x15)
        return int.from_bytes(self.f.read(3), 'big')

    def chain(self, start, count, consecutive):
        b = start
        for i in range(count):
            yield b
            b = b + 1 if consecutive else self.next_block(b)

    def entries(self):
        data = b''.join(self.block(b) for b in self.chain(self.ft_block, self.ft_count, False))
        out = []
        for i in range(0, len(data), 0x40):
            e = data[i:i+0x40]
            flags = e[0x28]
            if flags & 0x3F == 0: continue
            out.append(dict(index=i // 0x40, name=e[:flags & 0x3F].decode('latin-1'),
                            dir=bool(flags & 0x80), consecutive=bool(flags & 0x40),
                            blocks=int.from_bytes(e[0x29:0x2C], 'little'),
                            start=int.from_bytes(e[0x2F:0x32], 'little'),
                            parent=struct.unpack('>h', e[0x32:0x34])[0],
                            size=struct.unpack('>I', e[0x34:0x38])[0]))
        return out

    def extract(self, outdir):
        ents = self.entries()
        by = {e['index']: e for e in ents}
        def path(e):
            parts = [e['name']]
            while e['parent'] != -1:
                e = by[e['parent']]; parts.append(e['name'])
            return Path(*reversed(parts))
        for e in ents:
            p = Path(outdir) / path(e)
            if e['dir']: p.mkdir(parents=True, exist_ok=True); continue
            p.parent.mkdir(parents=True, exist_ok=True)
            left = e['size']
            with open(p, 'wb') as o:
                for b in self.chain(e['start'], e['blocks'], e['consecutive']):
                    d = self.block(b)[:min(left, 0x1000)]
                    o.write(d); left -= len(d)
            print(f'{e["size"]:>11}  {path(e)}')

if __name__ == '__main__':
    s = Stfs(sys.argv[1])
    print('#', s.magic, repr(s.name), hex(s.header_size), 'shift', s.shift)
    s.extract(sys.argv[2])
