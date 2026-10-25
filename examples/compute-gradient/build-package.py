"""Build a version 4 compute + storage-buffer wallpaper with no external dependencies."""
import argparse
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from package_tools import write_package


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    manifest = {
        "format_version": 1, "id": "example.compute-gradient", "name": "Compute Gradient",
        "author": "Wisp", "description": "Two compute passes, shared storage and fragment presentation",
        "preview": None, "program": "program.json", "minimum_wisp_version": "0.1.0",
        "gpu_profile": 1, "tags": ["example", "compute"],
    }
    program = {
        "version": 4, "shader_abi_version": 3,
        "parameters": [
            {"id": "speed", "default": {"type": "float", "value": 1}, "min": 0, "max": 4},
            {"id": "mirror", "default": {"type": "float", "value": 0.3}, "min": 0, "max": 1},
        ],
        "resources": [
            {"type": "storage_buffer", "id": "colors", "elements": 256 * 256},
            {"type": "render_target", "id": "generated", "width": 256, "height": 256},
            {"type": "render_target", "id": "mixed", "width": 256, "height": 256},
        ],
        "passes": [
            {"type": "compute", "shader": "shaders/generate.wgsl", "images": [],
             "buffers": [{"resource": "colors", "writable": True}],
             "output": "generated", "dispatch": [32, 32, 1]},
            {"type": "compute", "shader": "shaders/composite.wgsl", "images": ["generated"],
             "buffers": [{"resource": "colors", "writable": False}],
             "output": "mixed", "dispatch": [32, 32, 1]},
            {"type": "fragment", "shader": "shaders/present.wgsl", "background": "mixed",
             "output": "presentation"},
        ],
    }
    files = {"manifest.json": json.dumps(manifest, indent=2).encode(),
             "program.json": json.dumps(program, indent=2).encode()}
    for name in ("generate", "composite", "present"):
        files[f"shaders/{name}.wgsl"] = (HERE / f"{name}.wgsl").read_bytes()
    write_package(args.output, files)
    print(f"Created {args.output}")


if __name__ == "__main__":
    main()
