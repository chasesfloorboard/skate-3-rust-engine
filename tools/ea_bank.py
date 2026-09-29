"""Samples embedded in Skate 3 SPLC sound banks (data/audio/*.bnk).

vgmstream reads the .abk banks but not these. Each sample is an EA SNR header
(8 bytes: codec 3 = XMA, channels, rate; flags and sample count) followed by
one SNS block: flag/size (4), sample count (4), 4 unidentified bytes, then
the XMA packets (2048 bytes each, the last one trimmed). Samples are found by
scanning for headers whose block repeats the sample count, and are rewrapped
as XMA2 RIFF files that ffmpeg decodes."""
import re
import struct


def samples(bank: bytes):
    """(offset, rate, sample count, XMA packets) for each sample, in bank order."""
    for match in re.finditer(rb'\x03[\x00\x04]..', bank, re.S):
        at = match.start()
        rate = int.from_bytes(bank[at + 1:at + 4], 'big') & 0x3ffff
        if rate not in (22050, 24000, 32000, 44100, 48000):
            continue
        count = int.from_bytes(bank[at + 4:at + 8], 'big') & 0x1fffffff
        flag, size = bank[at + 8], int.from_bytes(bank[at + 9:at + 12], 'big')
        if flag not in (0, 0x80) or not 12 < size < 1 << 20:
            continue
        if int.from_bytes(bank[at + 12:at + 16], 'big') != count:
            continue
        yield at, rate, count, bank[at + 20:at + 8 + size]


def xma2_riff(packets: bytes, rate: int, count: int) -> bytes:
    """Mono XMA2 WAVE (XMA2WAVEFORMATEX) around whole 2048-byte packets."""
    data = packets + b'\x00' * (-len(packets) % 2048)
    fmt = struct.pack('<HHIIHHH', 0x166, 1, rate, rate * 2, 2048, 16, 34)
    fmt += struct.pack('<HIIIIIIIBBH', 1, 0, count, len(data), 0, count, 0, 0, 0, 4, 1)
    body = b'WAVE' + b'fmt ' + struct.pack('<I', len(fmt)) + fmt + b'data' + struct.pack('<I', len(data)) + data
    return b'RIFF' + struct.pack('<I', len(body)) + body
