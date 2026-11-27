"""Local-only import of a reviewed official Linux ARM64 AGY executable.

No installer execution, extraction into the host, download or redistribution.
The supplied digest is an integrity pin, not proof of publisher identity.
"""
import argparse
import hashlib
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "Vendor/litter-ish/build"
UBUNTU = BUILD / "ubuntu-base-24.04.5-base-arm64.tar.gz"
UBUNTU_SHA = "a91d5a93010193712d346d761372b7c9db6dfcf093893161c64ca107f05914f2"
PIN = "c8e9dcb954963b0d9f359b3ad9da871f19b28652"
IMPORT_SLOT = b"NEXUS_AGY_IMPORT_SLOT_V1\n"

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def validate_binary(path, expected):
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise ValueError("Expected lowercase SHA-256")
    if not path.is_file() or path.is_symlink() or not 64 <= path.stat().st_size <= 1024**3:
        raise ValueError("Expected regular executable file up to 1 GiB")
    if digest(path) != expected:
        raise ValueError("AGY digest mismatch")
    with path.open("rb") as stream:
        header = stream.read(64)
    if header[:7] != b"\x7fELF\x02\x01\x01" or struct.unpack_from("<HHI", header, 16) not in ((2,183,1),(3,183,1)):
        raise ValueError("Expected ELF64 little-endian AArch64 executable or PIE")

def overlay_archive(original, binary, output):
    # Preserve original tar metadata; never extract paths on the host.
    with tarfile.open(original, "r:gz") as src, tarfile.open(output, "x:gz") as dst:
        found_opt = False
        for member in src:
            p = PurePosixPath(member.name)
            if p.is_absolute() or ".." in p.parts:
                raise ValueError("Unexpected rootfs archive path")
            name = p.as_posix()
            if name == "opt":
                if not member.isdir() or found_opt:
                    raise ValueError("Invalid /opt directory")
                found_opt = True
            if name == "opt/nexus" or name.startswith("opt/nexus/"):
                raise ValueError("Refusing existing /opt/nexus")
            if member.isfile():
                with src.extractfile(member) as stream:
                    dst.addfile(member, stream)
            else:
                dst.addfile(member)
        if not found_opt:
            info = tarfile.TarInfo("./opt")
            info.type = tarfile.DIRTYPE
            info.mode = 0o755
            dst.addfile(info)
        info = tarfile.TarInfo("./opt/nexus")
        info.type = tarfile.DIRTYPE
        info.mode = 0o755
        dst.addfile(info)
        info = tarfile.TarInfo("./opt/nexus/agy")
        info.mode = 0o755
        info.size = binary.stat().st_size
        with binary.open("rb") as stream:
            dst.addfile(info, stream)

def overlay_import_slot(original, output):
    # A non-executable-content marker in a guest executable metadata slot.
    # No AGY binary is included; native import replaces bytes before boot.
    with tempfile.TemporaryDirectory(prefix="agy-slot-") as tmp:
        marker = Path(tmp) / "slot"
        marker.write_bytes(IMPORT_SLOT)
        overlay_archive(original, marker, output)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--confirm-local-use", action="store_true", required=True)
    args = parser.parse_args()
    target = BUILD / "agy-fs"
    if target.exists() or target.is_symlink():
        raise SystemExit("Refusing to overwrite existing agy-fs")
    if subprocess.check_output(["git", "-C", str(ROOT / "Vendor/litter-ish"), "rev-parse", "HEAD"], text=True).strip() != PIN:
        raise SystemExit("Unexpected runtime commit")
    subprocess.run(["python3", str(ROOT / "scripts/apply-runtime-patches.py"), str(ROOT / "Vendor/litter-ish")], check=True)
    if digest(UBUNTU) != UBUNTU_SHA:
        raise SystemExit("Ubuntu archive digest mismatch")
    converter = BUILD / "host-fakefsify/tools/fakefsify"
    if not os.access(converter, os.X_OK):
        raise SystemExit("Build the pinned runtime first")
    # Copy then validate the actual staged bytes used for import.
    validate_binary(args.binary, args.sha256)
    with tempfile.TemporaryDirectory(prefix="agy-import-", dir=BUILD) as tmp:
        stage = Path(tmp)
        binary = stage / "agy"
        shutil.copyfile(args.binary, binary)
        validate_binary(binary, args.sha256)
        archive = stage / "rootfs.tar.gz"
        overlay_archive(UBUNTU, binary, archive)
        fs = stage / "fs"
        subprocess.run([str(converter), str(archive), str(fs)], check=True)
        subprocess.run(["sqlite3", str(fs / "meta.db"), "PRAGMA wal_checkpoint(TRUNCATE);"], check=True)
        for suffix in ("-shm", "-wal"):
            (fs / ("meta.db" + suffix)).unlink(missing_ok=True)
        if digest(fs / "data/opt/nexus/agy") != args.sha256:
            raise SystemExit("Imported executable digest mismatch")
        (fs / "agy.sha256").write_text(args.sha256 + "\n")
        (fs / "provenance.txt").write_text("ubuntu_sha256=" + UBUNTU_SHA + "\nagy_sha256=" + args.sha256 + "\nsource=developer-reviewed official CLI\n")
        if target.exists() or target.is_symlink():
            raise SystemExit("Target appeared during import")
        fs.rename(target)
    print("Prepared local-only", target / "data")

if __name__ == "__main__":
    main()
