"""Build the two-image ABI 2 example using only Python's standard library."""
import argparse
import json
from pathlib import Path
import struct
import zlib
import zipfile

HERE = Path(__file__).resolve().parent


def accent_png():
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data)))

    size = 256
    rows = bytearray()
    for y in range(size):
        rows.append(0)  # PNG filter: none
        for x in range(size):
            stripe = ((x + y) // 32) % 2
            rows.extend((40 + 100 * stripe, 60 + y // 2, 180 + x // 4, 255))
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">2I5B", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new .wisp destination")
    args = parser.parse_args()
    manifest = {
        "format_version": 1, "id": "example.texture-blend", "name": "Texture Blend",
        "author": "Wisp", "description": "Two images with configurable blending and tint",
        "preview": "assets/background.png", "program": "program.json",
        "minimum_wisp_version": "0.1.0", "gpu_profile": 1, "tags": ["example"],
    }
    program = {
        "version": 2, "shader_abi_version": 2,
        "resources": [
            {"type": "image", "id": "accent", "path": "assets/accent.png"},
            {"type": "image", "id": "base", "path": "assets/background.png"},
        ],
        "parameters": [
            {"id": "blend", "default": {"type": "float", "value": 0.35}, "min": 0, "max": 1},
            {"id": "tint", "default": {"type": "color", "value": [1, 1, 1, 1]}},
            {"id": "animate", "default": {"type": "boolean", "value": True}},
            {"id": "speed", "default": {"type": "float", "value": 1}, "min": 0, "max": 4},
            {"id": "offset", "default": {"type": "vector2", "value": [0, 0]}},
        ],
        "passes": [{"type": "fragment", "shader": "shaders/pass.wgsl",
                    "background": "base", "images": ["accent"], "output": "presentation"}],
    }
    assets = {
        "manifest.json": json.dumps(manifest, indent=2).encode(),
        "program.json": json.dumps(program, indent=2).encode(),
        "shaders/pass.wgsl": (HERE / "wallpaper.wgsl").read_bytes(),
        "assets/background.png": (HERE.parent / "shader-basic/background.png").read_bytes(),
        "assets/accent.png": accent_png(),
    }
    with zipfile.ZipFile(args.output, "x", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in assets.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data)
    print(f"Created {args.output}")


if __name__ == "__main__":
    main()
