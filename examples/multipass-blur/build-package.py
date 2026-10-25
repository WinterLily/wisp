"""Build a portable three-pass blur example using Python's standard library."""
import argparse
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from package_tools import rgba_png, write_package


def image_png():
    def pixel_at(x, y):
        bright = ((x // 16) + (y // 16)) % 2
        return 30 + 180 * bright, 40 + x // 2, 100 + y // 2, 255

    return rgba_png(256, 256, pixel_at)


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
    write_package(args.output, assets)
    print(f"Created {args.output}")


if __name__ == "__main__":
    main()
