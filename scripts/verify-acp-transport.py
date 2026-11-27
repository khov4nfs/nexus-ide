"""Require complete multi-call transport fixture evidence; never AGY evidence."""
import json
from pathlib import Path
import sys

def require(condition, message):
    if not condition:
        raise SystemExit(message)

require(len(sys.argv) == 2 or (len(sys.argv) == 3 and sys.argv[2] in ("--cancel", "--chat")), "Usage: REPORT [--cancel|--chat]")
cancel = "--cancel" in sys.argv[2:]
chat = "--chat" in sys.argv[2:]
p = Path(sys.argv[1])
require(p.is_file(), "Missing transport fixture report")
r = json.loads(p.read_text())
require(r.get("ok") is True and r.get("fixtureOnly") is True, "Transport fixture failed")
i = r["initialize"]
require(i.get("credentialMode") == "gemini-api-key", "Missing synthetic credential handoff evidence")
require("NEXUS_TEST_ONLY_NOT_A_REAL_KEY" not in json.dumps(r), "Credential leaked into report")
require(i.get("ok") is True and i.get("active") is True and i.get("response") == {
    "jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1,
        "agentInfo": {"name": "Nexus Transport Fixture", "version": "test"}}}, "Invalid initialize fixture")
n = r["newSession"]
require(n.get("ok") is True and n.get("active") is True and n.get("sessionCreated") is True and n.get("responseId") == 2, "Missing second request evidence")
require(r["duplicate"].get("ok") is False and r["duplicate"].get("active") is True, "Duplicate request not rejected")
require(r["beforeSession"].get("ok") is False and r["beforeSession"].get("active") is True, "Prompt allowed before session")
p = r["prompt"]
require(r.get("chatMode") is chat and r.get("cancelMode") is cancel and r["idleCancel"].get("ok") is False and r["finishedProgress"].get("running") is False, "Wrong mode or prompt state")
require(p.get("ok") is True and p.get("active") is True and p.get("responseId") == 3, "Prompt failed")
if chat:
    require(r.get("progressSeen") is True and p.get("chatCompleted") is True and p.get("chunkCount") == 2 and p.get("stopReason") == "end_turn", "Missing streaming chat evidence")
    require(r["finishedProgress"].get("text") == "Xin chào Nexus!", "Incorrect final chat text")
elif cancel:
    require(r.get("progressSeen") is True and r["cancelRequest"].get("ok") is True and r["duplicateCancel"].get("ok") is False, "Missing live progress or cancel request")
    require(p.get("cancelled") is True and p.get("stopReason") == "cancelled" and p.get("chunkCount") == 1 and "marker" not in p, "Missing cancellation acknowledgement")
else:
    require(p.get("promptVerified") is True and p.get("chunkCount") == 2 and p.get("stopReason") == "end_turn" and p.get("marker") == "NEXUS_ACP_OK", "Missing prompt chunk evidence")
require(r["duplicatePrompt"].get("ok") is False and r["duplicatePrompt"].get("active") is True, "Duplicate prompt not rejected")
c = r["close"]
require(c.get("ok") is True and c.get("active") is False and c.get("transportClosed") is True and c.get("shutdownClean") is True, "Close or shutdown failed")
require(r["afterClose"].get("ok") is False and r["afterClose"].get("active") is False, "Request accepted after close")
print("Persistent local ACP transport fixture verified; AGY not executed")
