import struct
import tempfile
import unittest
from pathlib import Path
import zipfile
import zlib

from package_tools import rgba_png, write_package


class PackageToolsTests(unittest.TestCase):
    def test_png_chunks_crc_and_rows_preserve_rgba(self):
        image = rgba_png(2, 1, lambda x, y: (x, y, 128, 255))
        self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
        chunks = {}
        offset = 8
        while offset < len(image):
            length = struct.unpack(">I", image[offset:offset + 4])[0]
            kind = image[offset + 4:offset + 8]
            payload = image[offset + 8:offset + 8 + length]
            checksum = struct.unpack(">I", image[offset + 8 + length:offset + 12 + length])[0]
            self.assertEqual(checksum, zlib.crc32(kind + payload))
            chunks[kind] = payload
            offset += length + 12
        self.assertEqual(struct.unpack(">2I5B", chunks[b"IHDR"]), (2, 1, 8, 6, 0, 0, 0))
        self.assertEqual(zlib.decompress(chunks[b"IDAT"]), bytes([0, 0, 0, 128, 255, 1, 0, 128, 255]))
        self.assertEqual(chunks[b"IEND"], b"")

    def test_packages_are_deterministic_and_do_not_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = (Path(directory) / name for name in ["first.wisp", "second.wisp"])
            write_package(first, {"z": b"last", "a": b"first"})
            write_package(second, {"a": b"first", "z": b"last"})
            original = first.read_bytes()
            self.assertEqual(original, second.read_bytes())
            with zipfile.ZipFile(first) as archive:
                self.assertEqual(archive.read("a"), b"first")
                self.assertEqual(archive.getinfo("a").date_time, (2026, 1, 1, 0, 0, 0))
            with self.assertRaises(FileExistsError):
                write_package(first, {"replacement": b"wrong"})
            self.assertEqual(first.read_bytes(), original)

    def test_failed_encoding_removes_partial_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "failed.wisp"
            with self.assertRaises(TypeError):
                write_package(destination, {"first": b"ok", "invalid": object()})
            self.assertFalse(destination.exists())


if __name__ == "__main__":
    unittest.main()
