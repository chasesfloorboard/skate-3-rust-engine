"""Classic EA BIGF reader/extractor: python bigf.py ARCHIVE [OUTDIR]"""
import struct, sys
from pathlib import Path

def entries(path):
    with open(path, 'rb') as f:
        h = f.read(16)
        assert h[:4] in (b'BIGF', b'BIG4'), h[:4]
        count, hsize = struct.unpack('>II', h[8:16])
        data = f.read(hsize - 16)
    out, p = [], 0
    for _ in range(count):
        off, size = struct.unpack('>II', data[p:p+8]); p += 8
        end = data.index(b'\0', p)
        out.append((data[p:end].decode('latin-1').replace('\\', '/'), off, size)); p = end + 1
    return out

def extract(path, outdir):
    with open(path, 'rb') as f:
        for name, off, size in entries(path):
            dst = Path(outdir) / name
            dst.parent.mkdir(parents=True, exist_ok=True)
            f.seek(off); dst.write_bytes(f.read(size))

if __name__ == '__main__':
    if len(sys.argv) > 2: extract(sys.argv[1], sys.argv[2])
    else:
        for n, o, s in entries(sys.argv[1]): print(s, n)
