"""Independent completed-report verification for Ubuntu compatibility."""
import json
from pathlib import Path
import re
import sys

def require(ok, message):
    if not ok:
        raise SystemExit(message)

require(len(sys.argv) in (2, 3) and (len(sys.argv) == 2 or sys.argv[2] == "--agy"), "Usage: verify-glibc-evidence.py REPORT [--agy]")
agy = len(sys.argv) == 3
p = Path(sys.argv[1])
require(p.is_file(), "Missing completed compatibility evidence")
r = json.loads(p.read_text())
require(r.get("ok") is True and r.get("stage") == "compatibility" and r.get("agyRequested") is agy and "shutdownError" in r and r["shutdownError"] is None, "Compatibility or shutdown failed")
rows = r.get("checks", [])
require([row.get("name") for row in rows] == (["glibc_compat", "agy_binary_digest", "agy_acp_initialize"] if agy else ["glibc_compat"]) and all(row.get("passed") is True for row in rows), "Incomplete checks")
e = rows[0]["evidence"]
require(e.get("closed") is True and e.get("exitCode") == 0 and e.get("termSignal") == 0 and not e.get("pty") and rows[0].get("error") is None, "glibc process failed")
m = re.fullmatch(r"aarch64\nglibc (\d+)\.(\d+)\nNEXUS_GLIBCXX_OK\n", e.get("stdout", ""))
require(m is not None and tuple(map(int, m.groups())) >= (2, 28), "glibc version or C++ runtime evidence mismatch")
if agy:
    import os
    expected = os.environ.get("NEXUS_AGY_SHA256", "")
    require(re.fullmatch(r"[0-9a-f]{64}", expected) is not None and rows[1].get("sha256") == expected and rows[1].get("closed") is True and rows[1].get("error") is None, "Missing executable digest evidence")
    e = rows[2]["evidence"]
    require(rows[2].get("closed") is True and e.get("protocolVersion") == 1 and "antigravity" in e.get("agentName", "").lower(), "Invalid AGY handshake")
print("Completed Ubuntu compatibility evidence verified" + (" with real AGY initialize" if agy else "; AGY not executed"))
