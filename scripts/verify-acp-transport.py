"""Require complete multi-call transport fixture evidence; never AGY evidence."""
import json
from pathlib import Path
import sys

def require(condition, message):
    if not condition:
        raise SystemExit(message)

p = Path(sys.argv[1])
require(p.is_file(), "Missing transport fixture report")
r = json.loads(p.read_text())
require(r.get("ok") is True and r.get("fixtureOnly") is True, "Transport fixture failed")
i = r["initialize"]
require(i.get("ok") is True and i.get("active") is True and i.get("response") == {
    "jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1,
        "agentInfo": {"name": "Nexus Transport Fixture", "version": "test"}}}, "Invalid initialize fixture")
n = r["newSession"]
require(n.get("ok") is True and n.get("active") is True and n.get("sessionCreated") is True and n.get("responseId") == 2, "Missing second request evidence")
require(r["duplicate"].get("ok") is False and r["duplicate"].get("active") is True, "Duplicate request not rejected")
c = r["close"]
require(c.get("ok") is True and c.get("active") is False and c.get("transportClosed") is True and c.get("shutdownClean") is True, "Close or shutdown failed")
require(r["afterClose"].get("ok") is False and r["afterClose"].get("active") is False, "Request accepted after close")
print("Persistent local ACP transport fixture verified; AGY not executed")
