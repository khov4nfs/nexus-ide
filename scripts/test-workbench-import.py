import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("workbench", Path(__file__).with_name("prepare-workbench-rootfs.py"))
workbench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workbench)

def add(t, name, data=None, link=None):
    m = tarfile.TarInfo(name)
    m.mode = 0o755
    if link is not None:
        m.type = tarfile.SYMTYPE
        m.linkname = link
        t.addfile(m)
    elif data is None:
        m.type = tarfile.DIRTYPE
        t.addfile(m)
    else:
        m.size = len(data)
        t.addfile(m, io.BytesIO(data))

class OverlayTests(unittest.TestCase):
    def test_overlay_and_reject_escape(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            base = root / "base.tar.gz"
            with tarfile.open(base, "w:gz") as t:
                add(t, "./opt")
            for escape in (False, True):
                package = root / f"package-{escape}.tar.gz"
                prefix = workbench.PREFIX
                with tarfile.open(package, "w:gz") as t:
                    add(t, prefix)
                    add(t, prefix + "/bin")
                    add(t, prefix + "/bin/openvscode-server", b"fixture")
                    add(t, prefix + "/node", b"fixture")
                    add(t, prefix + "/LICENSE", b"fixture")
                    add(t, prefix + "/bin/link", link="../../outside" if escape else "../node")
                out = root / f"out-{escape}.tar.gz"
                if escape:
                    with self.assertRaises(ValueError):
                        workbench.overlay(base, package, out)
                else:
                    workbench.overlay(base, package, out)
                    with tarfile.open(out, "r:gz") as t:
                        p = "./" + workbench.DEST
                        self.assertEqual(t.extractfile(p + "/node").read(), b"fixture")
                        self.assertEqual(t.extractfile("./" + workbench.VSIX_SLOT).read(), workbench.VSIX_MARKER)
                        self.assertEqual(t.getmember(p + "/bin/link").linkname, "../node")

if __name__ == "__main__":
    unittest.main()
