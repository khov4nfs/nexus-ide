"""Syntax checks before expensive guest builds; no guest or network needed."""
from pathlib import Path
import re
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
sources = sorted((root / "RuntimeBridge/src").glob("*.rs"))
if not sources:
    raise SystemExit("Missing Rust sources")
for source in sources:
    subprocess.run(
        ["rustfmt", "--edition", "2021", "--config", "skip_children=true", "--emit", "stdout", str(source)],
        stdout=subprocess.DEVNULL, check=True,
    )
probe = (root / "RuntimeBridge/src/process_probe.rs").read_text()
scripts = re.findall(r'const PACKAGE_SCRIPT: &str = r#"(.*?)"#;', probe, re.S)
if len(scripts) != 1:
    raise SystemExit("Expected exactly one package shell fixture")
subprocess.run(["sh", "-n"], input=scripts[0], text=True, check=True)
for fixture in sorted((root / "RuntimeBridge/fixtures").glob("*.sh")):
    subprocess.run(["sh", "-n", str(fixture)], check=True)
subprocess.run(["bash", "-n", str(root / "scripts/build-guest-device.sh")], check=True)
for script in sorted((root / "scripts").glob("*.py")):
    compile(script.read_bytes(), str(script), "exec")
print("Rust, embedded shell, build shell and Python syntax checks passed")
