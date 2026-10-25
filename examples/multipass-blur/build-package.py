"""Build a portable three-pass blur example using Python's standard library."""
import argparse
import json
from pathlib import Path
import struct
import zlib
import zipfile

HERE = Path(__file__).resolve().parent


def image_png():
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data)))

    size = 256
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            bright = ((x // 16) + (y // 16)) % 2
            rows.extend((30 + 180 * bright, 40 + x // 2, 100 + y // 2, 255))
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">2I5B", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new .wisp destination")
    args = parser.parse_args()
    manifest = {
        "format_version": 1, "id": "example.multipass-blur", "name": "Multipass Blur",
        "author": "Wisp", "description": "Animated comparison of original and separable blur",
        "preview": "assets/source.png", "program": "program.json",
        "minimum_wisp_version": "0.1.0", "gpu_profile": 1, "tags": ["example", "blur"],
    }
    program = {
        "version": 3, "shader_abi_version": 2,
        "resources": [
            {"type": "image", "id": "source", "path": "assets/source.png"},
            {"type": "render_target", "id": "horizontal", "width": 256, "height": 256},
            {"type": "render_target", "id": "blurred", "width": 256, "height": 256},
        ],
        "parameters": [
            {"id": "radius", "default": {"type": "float", "value": 3}, "min": 0, "max": 8},
            {"id": "strength", "default": {"type": "float", "value": 1}, "min": 0, "max": 1},
        ],
        "passes": [
            {"type": "fragment", "shader": "shaders/horizontal.wgsl", "background": "source",
             "output": {"target": "horizontal"}},
            {"type": "fragment", "shader": "shaders/vertical.wgsl", "background": "horizontal",
             "output": {"target": "blurred"}},
            {"type": "fragment", "shader": "shaders/composite.wgsl", "background": "blurred",
             "images": ["source"], "output": "presentation"},
        ],
    }
    assets = {
        "manifest.json": json.dumps(manifest, indent=2).encode(),
        "program.json": json.dumps(program, indent=2).encode(),
        "assets/source.png": image_png(),
    }
    for name in ("horizontal", "vertical", "composite"):
        assets[f"shaders/{name}.wgsl"] = (HERE / f"{name}.wgsl").read_bytes()
    with zipfile.ZipFile(args.output, "x", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in assets.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data)
    print(f"Created {args.output}")


if __name__ == "__main__":
    main()
