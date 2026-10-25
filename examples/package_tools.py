"""Standard-library encoding shared by the portable example builders."""

from pathlib import Path
import struct
import zlib
import zipfile


def rgba_png(width, height, pixel_at):
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data)))

    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows.extend(pixel_at(x, y))
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">2I5B", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def write_package(destination, files):
    destination = Path(destination)
    stream = destination.open("xb")
    try:
        with stream, zipfile.ZipFile(stream, "w") as archive:
            for name, data in sorted(files.items()):
                info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, data)
    except Exception:
        # Failed examples must not leave an apparently complete wallpaper behind.
        destination.unlink()
        raise
