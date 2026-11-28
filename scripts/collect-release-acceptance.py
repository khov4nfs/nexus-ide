"""Bind CI evidence to an exact structurally verified unsigned IPA.
Never equate a synthetic protocol peer or macOS guest test with iPhone
device, provider login, inference, licensing or production acceptance.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re

# Old independent verifiers and all detailed checks run earlier in the same
# workflow. This gate also refuses incomplete/failed evidence inventories.
def gates(variant):
    rows = {}
    for name in ("guest-evidence", "glibc-evidence"):
        rows[name] = ("macOS guest", {"ok": True, "shutdownError": None})
    for name in ("acp-evidence", "acp-cancel-evidence", "acp-chat-evidence"):
        rows[name] = ("synthetic ACP peer", {"ok": True, "fixtureOnly": True})
    for name in ("local-pty", "local-mcp", "local-call"):
        rows[name] = ("macOS guest local job", {"ok": True, "shutdownClean": True})
    rows["slot-evidence"] = ("Ubuntu ELF fixture, not AGY", {"ok": True, "shutdownClean": True, "digestVerified": True})
    for name in ("google", "google-edit", "snapshot", "mcp-session", "terminal", "codex-probe"):
        rows[name] = ("synthetic protocol peer", {"ok": True, "fixtureOnly": True, "shutdownClean": True})
    groups = {
        "acp-review": ("allow", "deny", "cancel"),
        "acp-hooks": ("success", "beforeFailure", "afterFailure"),
        "adapter": ("success", "resume", "unexpectedCallback", "unsupportedResume"),
        "acp-resume": ("create", "resume"),
        "codex-turn": ("success", "policy", "mismatch"),
        "claude": ("signedIn", "signedOut", "success", "error"),
        "conversation-codex": ("create", "resume"),
        "conversation-claude": ("create", "resume"),
        "lsp": ("success", "invalidUri", "invalidRange", "completion", "completionInvalid"),
        "transfer": ("push", "duplicate", "pull"),
        "batch": ("push", "duplicate", "pull", "missing"),
    }
    for group, phases in groups.items():
        for phase in phases:
            required = {"ok": True, "fixtureOnly": True, "shutdownClean": True, "phase": phase}
            if group.startswith("conversation-"):
                required["turnsCompleted"] = 2
            rows[f"{group}-{phase}"] = ("synthetic protocol peer", required)
    for kind in ("codex", "claude"):
        rows[f"cli-{kind}"] = ("Ubuntu ELF fixture, not provider CLI", {"ok": True, "shutdownClean": True})
    for phase in ("success", "failure", "main_failure"):
        rows[f"hooks-{phase}"] = ("macOS guest hook fixture", {"ok": True, "shutdownClean": True})
    for phase in ("write", "read", "scroll"):
        rows[f"persist-{phase}"] = ("macOS guest persistence", {"ok": True, "verified": True, "shutdownClean": True})
    if variant == "workbench":
        rows["workbench"] = ("macOS guest Chromium editor and synthetic VSIX activation", {"ok": True, "shutdownClean": True, "extensionInstallVerified": True, "browserRequested": True, "extensionActivationVerified": True, "editorVerified": True, "lspVerified": False})
    return rows

def read_json(path):
    if not path.is_file() or path.is_symlink() or path.stat().st_size > 2_000_000:
        raise ValueError(f"Missing or invalid evidence: {path.name}")
    data = path.read_bytes()
    obj = json.loads(data)
    if not isinstance(obj, dict):
        raise ValueError(f"Invalid evidence object: {path.name}")
    return obj, hashlib.sha256(data).hexdigest()

def equal(value, expected):
    # bool is an int subclass: do not accept 1 instead of true.
    return type(value) is type(expected) and value == expected

def collect(evidence, package, commit, variant):
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or variant not in ("ubuntu", "workbench"):
        raise ValueError("Invalid commit or variant")
    build, build_sha = read_json(package / "build.json")
    if build.get("commit") != commit or build.get("variant") != variant or build.get("physicalDeviceVerified") is not False:
        raise ValueError("Build identity or scope mismatch")
    name = build.get("file")
    if name != f"NexusIDE-{variant}-{commit[:12]}-unsigned.ipa":
        raise ValueError("Unexpected IPA name")
    archive = package / name
    if not archive.is_file() or archive.is_symlink():
        raise ValueError("IPA missing")
    with archive.open("rb") as stream:
        sha = hashlib.file_digest(stream, "sha256").hexdigest()
    if sha != build.get("sha256") or archive.stat().st_size != build.get("bytes"):
        raise ValueError("IPA integrity mismatch")
    reports = []
    for name, (scope, required) in gates(variant).items():
        path = evidence / ("nexus-" + name + ".json")
        obj, digest = read_json(path)
        for key, expected in required.items():
            if key not in obj or not equal(obj[key], expected):
                raise ValueError(f"Failed evidence: {path.name} / {key}")
        reports.append({"file": path.name, "sha256": digest, "scope": scope, "passed": True})
    return {
        "schemaVersion": 1, "commit": commit, "variant": variant,
        "ipa": archive.name, "ipaSha256": sha, "buildJsonSha256": build_sha,
        "ciAcceptancePassed": True, "checks": reports,
        "physicalDeviceVerified": False, "realProviderAuthVerified": False,
        "realProviderInferenceVerified": False, "productionReady": False,
        "remaining": ["signed physical iPhone acceptance", "real imported provider binaries and auth/inference",
                      "workbench editor/extension activation and LSP", "license and source obligations",
                      "full-system VM/Docker not implemented"],
    }

if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--evidence", type=Path, required=True)
    p.add_argument("--package", type=Path, required=True)
    p.add_argument("--commit", required=True)
    p.add_argument("--variant", required=True, choices=("ubuntu", "workbench"))
    a = p.parse_args()
    report = collect(a.evidence, a.package, a.commit, a.variant)
    with (a.package / "acceptance.json").open("x") as out:
        json.dump(report, out, indent=2, sort_keys=True)
        out.write("\n")
    print(f"Verified {len(report['checks'])} CI evidence files; physical iPhone and production not verified")
