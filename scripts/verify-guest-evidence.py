"""Fail closed if the emulator exits before returning a complete report."""
import json
import re
from pathlib import Path
import sys

def require(condition, message):
    if not condition:
        raise SystemExit(message)

require(len(sys.argv) in (2, 3) and (len(sys.argv) == 2 or sys.argv[2] in ('--packages', '--toolchains')), 'Usage: verify-guest-evidence.py REPORT [--packages|--toolchains]')
packages = len(sys.argv) == 3
toolchains = packages and sys.argv[2] == '--toolchains'
path = Path(sys.argv[1])
require(path.is_file(), "Missing completed guest evidence: possible premature host exit")
report = json.loads(path.read_text())
require(report.get("ok") is True and report.get("stage") == "probe" and "shutdownError" in report and report["shutdownError"] is None, "Guest probe or shutdown failed")
rows = report.get("checks", [])
names = ["architecture", "stdout", "shell_exit", "stdio_roundtrip", "pty_resize", "process_cancel"]
if packages:
    names.append("apk_install_jq")
if toolchains:
    names.append("development_toolchains")
require(report.get("packageRequested") is packages and report.get("toolchainsRequested") is toolchains, "Wrong probe mode")
require([r.get("name") for r in rows] == names and all(r.get("passed") is True for r in rows), "Incomplete or failed checks")
for row, code, marker in zip(rows[:3], [0,0,7], ["aarch64","NEXUS_GUEST_OK","NEXUS_SHELL_OK"]):
    require(row.get("exitCode") == code and row.get("output","").strip() == marker, "Invalid basic command evidence")
stdio, pty, cancel = [row["evidence"] for row in rows[3:6]]
require(all(e.get("closed") is True for e in (stdio,pty,cancel)), "Process not closed")
require(stdio.get("exitCode") == 0 and "reply:nexus-ping" in stdio.get("stdout","") and "NEXUS_OUT_READY" in stdio.get("stdout","") and "NEXUS_ERR_READY" in stdio.get("stderr","") and "NEXUS_ERR_READY" not in stdio.get("stdout","") and "NEXUS_OUT_READY" not in stdio.get("stderr",""), "Invalid stdio evidence")
require(pty.get("exitCode") == 0 and any(line.strip()=="33 111" for line in pty.get("pty","").splitlines()) and "NEXUS_PTY_TAIL" in pty.get("pty",""), "Invalid PTY evidence")
require(cancel.get("termSignal") == 9, "Invalid cancellation evidence")
if packages:
    package = rows[6]
    require(package.get("repository") == "https://dl-cdn.alpinelinux.org/alpine/v3.21/main" and package.get("error") is None, "Invalid repository or package error")
    e = package["evidence"]
    lines = e.get("stdout", "").splitlines()
    require(e.get("closed") is True and e.get("exitCode") == 0 and e.get("termSignal") == 0 and not e.get("pty"), "Package process failed")
    require("NEXUS_PACKAGE_ABSENT" in lines and "NEXUS_PACKAGE_INSTALLED" in lines and lines.index("NEXUS_PACKAGE_ABSENT") < lines.index("NEXUS_PACKAGE_INSTALLED"), "Missing install transition")
    tail = lines[lines.index("NEXUS_PACKAGE_INSTALLED")+1:]
    require(len(tail) == 3 and tail[-1] == "NEXUS_JQ_OK", "Missing version or execution evidence")
    pkg = re.fullmatch(r"NEXUS_APK_VERSION=(jq-[0-9][A-Za-z0-9._+\-]*)-r[0-9]+", tail[0])
    require(pkg is not None and tail[1] == "NEXUS_BINARY_VERSION=" + pkg.group(1), "Installed package and executable versions must match")
if toolchains:
    row = rows[7]
    e = row["evidence"]
    require(row.get("error") is None and e.get("closed") is True and e.get("exitCode") == 0 and e.get("termSignal") == 0 and not e.get("pty"), "Toolchain process failed")
    require("NEXUS_GUEST_ENTROPY_OK" in e.get("stderr", "").splitlines(), "Missing guest entropy evidence")
    lines = e.get("stdout", "").splitlines()
    require(len(lines) == 13 and lines[0] == "NEXUS_TOOLCHAINS_ABSENT" and lines[9:11] == ["NEXUS_NODE_DEPENDENCY_OK", "NEXUS_PYTHON_TEST_OK"] and lines[12] == "NEXUS_TOOLCHAINS_OK", "Missing toolchain workflow evidence")
    for line, name in zip(lines[1:5], ("nodejs", "npm", "python3", "git")):
        require(re.fullmatch(r"NEXUS_TOOLCHAIN_APK=" + name + r"-[0-9][A-Za-z0-9._+\-]*-r[0-9]+", line) is not None, "Invalid installed package evidence: " + name)
    for line, prefix in zip(lines[5:9], ("NEXUS_NODE_VERSION=v", "NEXUS_NPM_VERSION=", "NEXUS_PYTHON_VERSION=Python ", "NEXUS_GIT_VERSION=git version ")):
        require(line.startswith(prefix) and line[len(prefix):][:1] in tuple("0123456789"), "Invalid binary version: " + prefix)
    require(re.fullmatch(r"NEXUS_GIT_COMMIT=[0-9a-f]{40}", lines[11]) is not None, "Missing Git commit")
print(f"All {len(names)} guest checks and shutdown verified from completed evidence")
