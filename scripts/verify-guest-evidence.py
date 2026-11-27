"""Fail closed if the emulator exits before returning a complete report."""
import json
from pathlib import Path
import sys

def require(condition, message):
    if not condition:
        raise SystemExit(message)

path = Path(sys.argv[1])
require(path.is_file(), "Missing completed guest evidence: possible premature host exit")
report = json.loads(path.read_text())
require(report.get("ok") is True and report.get("stage") == "probe" and "shutdownError" in report and report["shutdownError"] is None, "Guest probe or shutdown failed")
rows = report.get("checks", [])
names = ["architecture", "stdout", "shell_exit", "stdio_roundtrip", "pty_resize", "process_cancel"]
require([r.get("name") for r in rows] == names and all(r.get("passed") is True for r in rows), "Incomplete or failed checks")
for row, code, marker in zip(rows[:3], [0,0,7], ["aarch64","NEXUS_GUEST_OK","NEXUS_SHELL_OK"]):
    require(row.get("exitCode") == code and row.get("output","").strip() == marker, "Invalid basic command evidence")
stdio, pty, cancel = [row["evidence"] for row in rows[3:]]
require(all(e.get("closed") is True for e in (stdio,pty,cancel)), "Process not closed")
require(stdio.get("exitCode") == 0 and "reply:nexus-ping" in stdio.get("stdout","") and "NEXUS_OUT_READY" in stdio.get("stdout","") and "NEXUS_ERR_READY" in stdio.get("stderr","") and "NEXUS_ERR_READY" not in stdio.get("stdout","") and "NEXUS_OUT_READY" not in stdio.get("stderr",""), "Invalid stdio evidence")
require(pty.get("exitCode") == 0 and any(line.strip()=="33 111" for line in pty.get("pty","").splitlines()), "Invalid PTY evidence")
require(cancel.get("termSignal") == 9, "Invalid cancellation evidence")
print("All six guest checks and shutdown verified from completed evidence")
