"""Regenerate the original gradient texture fixture using only Python's stdlib."""
import math
import pathlib
import struct
import zlib

size = 256
rows = bytearray()
for y in range(size):
    rows.append(0)
    for x in range(size):
        u, v = x / (size - 1), y / (size - 1)
        teal = math.exp(-((u - 0.25) ** 2 + (v - 0.3) ** 2) * 9)
        violet = math.exp(-((u - 0.78) ** 2 + (v - 0.7) ** 2) * 7)
        ribbon = (0.5 + 0.5 * math.sin(u * 14 + v * 9 + math.sin(v * 8))) ** 12
        rows.extend(min(255, int(c)) for c in (
            9 + 90 * violet + 28 * ribbon,
            15 + 155 * teal + 30 * violet + 25 * ribbon,
            30 + 155 * teal + 145 * violet + 30 * ribbon,
        ))

def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))

png = b'\x89PNG\r\n\x1a\n'
png += chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 2, 0, 0, 0))
png += chunk(b'IDAT', zlib.compress(rows))
png += chunk(b'IEND', b'')
pathlib.Path(__file__).with_name('background.png').write_bytes(png)
