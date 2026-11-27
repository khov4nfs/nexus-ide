"""Test the archive importer without downloading or executing any AGY binary."""
import importlib.util
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("prepare", Path(__file__).with_name("prepare-agy-rootfs.py"))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)

class ImportTests(unittest.TestCase):
    def test_digest_architecture_and_overlay(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            binary = d / "fixture"
            # Synthetic ELF header for importer unit tests ONLY, not executable.
            header = bytearray(64)
            header[:7] = b"\x7fELF\x02\x01\x01"
            struct.pack_into("<HHI", header, 16, 3, 183, 1)
            binary.write_bytes(header)
            sha = prepare.digest(binary)
            prepare.validate_binary(binary, sha)
            with self.assertRaises(ValueError):
                prepare.validate_binary(binary, "0" * 64)
            src = d / "root.tar.gz"
            with tarfile.open(src, "w:gz") as archive:
                opt = tarfile.TarInfo("./opt")
                opt.type = tarfile.DIRTYPE
                archive.addfile(opt)
            dst = d / "out.tar.gz"
            prepare.overlay_archive(src, binary, dst)
            with tarfile.open(dst, "r:gz") as archive:
                info = archive.getmember("./opt/nexus/agy")
                self.assertEqual(info.mode, 0o755)
                self.assertEqual(archive.extractfile(info).read(), bytes(header))
                for name, marker in prepare.CLI_SLOTS.items():
                    slot = archive.getmember("./opt/nexus/" + name)
                    self.assertEqual(slot.mode, 0o755)
                    self.assertEqual(archive.extractfile(slot).read(), marker)
            with self.assertRaises(ValueError):
                prepare.overlay_archive(dst, binary, d / "collision.tar.gz")
            struct.pack_into("<H", header, 18, 62)
            binary.write_bytes(header)
            with self.assertRaises(ValueError):
                prepare.validate_binary(binary, prepare.digest(binary))

if __name__ == "__main__":
    unittest.main()
