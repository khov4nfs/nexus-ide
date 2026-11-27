import importlib.util
from pathlib import Path
import plistlib
import struct
import tempfile
import unittest
import zipfile

spec=importlib.util.spec_from_file_location("ipa",Path(__file__).with_name("package-ipa.py"))
ipa=importlib.util.module_from_spec(spec)
spec.loader.exec_module(ipa)

def executable(platform):
    return struct.pack("<8I",0xfeedfacf,0x0100000c,0,2,1,24,0,0)+struct.pack("<6I",0x32,24,platform,0,0,0)

class Tests(unittest.TestCase):
    def test_platform(self):
        ipa.validate_macho(executable(2))
        for data in (executable(7), executable(2)[:-1], b"ELF"):
            with self.assertRaises(ValueError):
                ipa.validate_macho(data)

    def test_ipa_layout(self):
        with tempfile.TemporaryDirectory() as d:
            for variant,platform,valid in (("native",2,True),("native",7,False),("ubuntu",2,False)):
                path=Path(d)/f"{variant}-{platform}.ipa"
                prefix="Payload/NexusIDE.app/"
                with zipfile.ZipFile(path,"w") as z:
                    z.writestr(prefix+"Info.plist",plistlib.dumps({
                        "CFBundleIdentifier":ipa.VARIANTS[variant],"CFBundleSupportedPlatforms":["iPhoneOS"],
                        "MinimumOSVersion":"17.0","CFBundleExecutable":"NexusIDE",
                        "CFBundleShortVersionString":"0.5.0","CFBundleVersion":"1"}))
                    entry=zipfile.ZipInfo(prefix+"NexusIDE")
                    entry.create_system=3
                    entry.external_attr=0o100755<<16
                    z.writestr(entry,executable(platform))
                if valid:
                    self.assertEqual(ipa.verify(path,variant)["architecture"],"arm64")
                else:
                    with self.assertRaises(ValueError):
                        ipa.verify(path,variant)

if __name__=="__main__":
    unittest.main()
