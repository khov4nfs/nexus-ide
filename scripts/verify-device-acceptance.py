"""Offline owner-attested acceptance for one exact unsigned integrated IPA.

This verifies identity, completeness and evidence file hashes, NOT the truth of
operator observations. No device IDs, credentials, provider transcripts or
automatic upload. It never asserts independently verified production readiness.
"""
import argparse
import datetime
import hashlib
import importlib.util
import json
from pathlib import Path
import re

spec = importlib.util.spec_from_file_location("ci_acceptance", Path(__file__).with_name("collect-release-acceptance.py"))
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)

CHECKS = (
    "signed_install_launch", "native_save_relaunch", "ubuntu_pty_shutdown",
    "persistent_guest_relaunch", "mcp_stdio_manual_call",
    "agy_reviewed_import", "agy_google_auth_real", "agy_google_inference_real",
    "agy_google_saved_guest_fresh_login_resume_reset",
    "agy_gemini_auth_real", "agy_gemini_inference_real", "agy_cancel_and_reviewed_edit",
    "agy_session_resume_and_hooks", "codex_cli_auth_real", "codex_native_multiturn_resume",
    "claude_cli_auth_real", "claude_native_multiturn_resume",
    "claude_upstream_acp_real", "batch_transfer_conflict",
    "native_lsp_real_server", "live_lsp_refresh_real_server", "native_lsp_multi_document_real_server",
    "suspend_relaunch_resources", "license_source_notices_review",
)
WORKBENCH = ("wkwebview_loopback", "workbench_editor_save_relaunch",
             "workbench_real_extension_activation", "workbench_real_lsp", "workbench_real_lsp_diagnostics_completion_relaunch", "workbench_persistent_python_c_cpp_sessions", "workbench_cross_file_rename_and_refactoring_review")
VERSIONS = ("agy", "codex", "claude", "claudeACP", "languageServer")
FULL_VM = ("vm_kernel_boot_serial", "vm_disk_persistence_reboot", "vm_docker_engine_real", "vm_compose_volume_and_network", "vm_loopback_http_and_isolation", "vm_internet_dns_tls", "vm_registry_pull_arm64", "vm_clean_shutdown_resources")

def sha_file(path):
    if not path.is_file() or path.is_symlink():
        raise ValueError("Missing or invalid evidence file")
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()

def package_identity(package, expected_commit=None, expected_variant=None):
    build, build_sha = ci.read_json(package / "build.json")
    report, report_sha = ci.read_json(package / "acceptance.json")
    commit, variant = build.get("commit"), build.get("variant")
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit) or variant not in ("ubuntu", "workbench"):
        raise ValueError("Invalid package identity")
    if (expected_commit is not None and commit != expected_commit) or (expected_variant is not None and variant != expected_variant):
        raise ValueError("Package is not the expected source revision or variant")
    if build.get("physicalDeviceVerified") is not False or type(report.get("schemaVersion")) is not int or report["schemaVersion"] != 1:
        raise ValueError("Unexpected build or CI scope")
    name = f"NexusIDE-{variant}-{commit[:12]}-unsigned.ipa"
    if build.get("file") != name or report.get("ipa") != name:
        raise ValueError("IPA name mismatch")
    ipa = package / name
    sha = sha_file(ipa)
    if build.get("sha256") != sha or report.get("ipaSha256") != sha or build.get("bytes") != ipa.stat().st_size:
        raise ValueError("IPA hash or size mismatch")
    if report.get("commit") != commit or report.get("variant") != variant or report.get("buildJsonSha256") != build_sha or report.get("ciAcceptancePassed") is not True:
        raise ValueError("CI manifest identity mismatch")
    full_vm = build.get("fullVMIncluded", False)
    if type(full_vm) is not bool or report.get("fullVMIncluded", False) is not full_vm:
        raise ValueError("VM scope mismatch")
    expected = {"nexus-" + key + ".json": scope for key, (scope, _) in ci.gates(variant, full_vm).items()}
    rows = report.get("checks")
    if not isinstance(rows, list) or len(rows) != len(expected) or any(
        not isinstance(row, dict) or not isinstance(row.get("file"), str) or row.get("scope") != expected.get(row["file"]) or row.get("passed") is not True or
        not isinstance(row.get("sha256"), str) or not re.fullmatch(r"[0-9a-f]{64}", row["sha256"])
        for row in rows
    ) or {row.get("file") for row in rows} != set(expected):
        raise ValueError("Incomplete CI manifest")
    if any(report.get(k) is not False for k in ("physicalDeviceVerified", "realProviderAuthVerified", "realProviderInferenceVerified", "productionReady", "dockerEngineVerified", "composeVerified", "fullSystemVMVerified")):
        raise ValueError("Unexpected CI scope")
    identity = {"commit": commit, "variant": variant, "ipaSha256": sha, "ciManifestSha256": report_sha}
    if variant == "workbench":
        vsix_sha = sha_file(package / "nexus-real-pylsp.vsix")
        if report.get("workbenchRealVSIXSha256") != vsix_sha:
            raise ValueError("Real Workbench VSIX hash mismatch")
        identity["workbenchRealVSIXSha256"] = vsix_sha
    if full_vm:
        identity["fullVMIncluded"] = True
    return identity

def required(variant, full_vm=False):
    return CHECKS + (WORKBENCH if variant == "workbench" else ()) + (FULL_VM if full_vm else ())

def version_fields(identity):
    return VERSIONS + (("clangd",) if identity["variant"] == "workbench" else ()) + (("qemu", "linuxKernel", "dockerEngine", "dockerCompose") if identity.get("fullVMIncluded", False) else ())

def template(identity):
    return {
        "schemaVersion": 1, "package": identity,
        "operatorAcknowledgesManualReview": False,
        "device": {"model": "", "os": "", "installation": "", "testedDate": ""},
        "versions": {k: "" for k in version_fields(identity)},
        "checks": {k: {"passed": False, "evidenceSha256": ""} for k in required(identity["variant"], identity.get("fullVMIncluded", False))},
    }

def verify_release_template(identity, record):
    # Exact pristine template only: never ship an operator-completed record.
    expected = template(identity)
    if json.dumps(record, sort_keys=True) != json.dumps(expected, sort_keys=True):
        raise ValueError("Release device record is not an untouched template")


def validate(identity, record, evidence_dir):
    if set(record) != {"schemaVersion", "package", "operatorAcknowledgesManualReview", "device", "versions", "checks"} or type(record["schemaVersion"]) is not int or record["schemaVersion"] != 1 or record["package"] != identity:
        raise ValueError("Record schema or package identity mismatch")
    if record["operatorAcknowledgesManualReview"] is not True:
        raise ValueError("Explicit operator acknowledgement required")
    device = record["device"]
    if not isinstance(device, dict) or set(device) != {"model", "os", "installation", "testedDate"}:
        raise ValueError("Invalid device fields")
    if not isinstance(device["model"], str) or not re.fullmatch(r"iPhone [A-Za-z0-9 ,.-]{1,40}", device["model"]):
        raise ValueError("Use public iPhone model name, not serial or UDID")
    if not isinstance(device["os"], str) or not re.fullmatch(r"[0-9]{1,2}\.[0-9]{1,2}(\.[0-9]{1,2})?", device["os"]) or int(device["os"].split(".")[0]) < 17:
        raise ValueError("Invalid iOS version")
    if device["installation"] not in ("SideStore", "development", "ad-hoc", "TestFlight", "other-legitimate"):
        raise ValueError("Specify legitimate signing/install method")
    if not isinstance(device["testedDate"], str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", device["testedDate"]):
        raise ValueError("Invalid test date")
    datetime.date.fromisoformat(device["testedDate"])
    versions = record["versions"]
    if not isinstance(versions, dict) or set(versions) != set(version_fields(identity)) or any(
        not isinstance(v, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9 ._+@/-]{0,79}", v) for v in versions.values()
    ):
        raise ValueError("Specify tested component versions, never secrets")
    checks = record["checks"]
    if not isinstance(checks, dict) or set(checks) != set(required(identity["variant"], identity.get("fullVMIncluded", False))):
        raise ValueError("Incomplete or unexpected device checks")
    pending = []
    for key in required(identity["variant"], identity.get("fullVMIncluded", False)):
        item = checks[key]
        if not isinstance(item, dict) or set(item) != {"passed", "evidenceSha256"} or type(item["passed"]) is not bool:
            raise ValueError("Invalid check record")
        if not item["passed"]:
            pending.append(key)
            continue
        digest = item["evidenceSha256"]
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError("Missing evidence digest: " + key)
        path = evidence_dir / (key + ".txt")
        if not path.is_file() or path.is_symlink() or not (1 <= path.stat().st_size <= 65536) or sha_file(path) != digest:
            raise ValueError("Evidence missing or changed: " + key)
    return {
        "schemaVersion": 1, "package": identity, "scope": "operator-attested, not independently verified",
        "operatorReviewComplete": not pending, "pending": pending,
        "physicalDeviceIndependentlyVerified": False, "realProviderIndependentlyVerified": False,
        "dockerEngineVerified": False, "composeVerified": False, "fullSystemVMVerified": False,
        "productionReady": False,
    }

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--record", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--initialize", action="store_true")
    mode.add_argument("--verify-release-template", action="store_true")
    parser.add_argument("--expected-commit")
    parser.add_argument("--expected-variant", choices=("ubuntu", "workbench"))
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    identity = package_identity(args.package, args.expected_commit, args.expected_variant)
    if args.initialize:
        with args.record.open("x") as f:
            json.dump(template(identity), f, indent=2, sort_keys=True)
            f.write("\n")
    elif args.verify_release_template:
        if args.expected_commit is None or args.expected_variant is None:
            parser.error("Release verification requires expected commit and variant")
        record, _ = ci.read_json(args.record)
        verify_release_template(identity, record)
        # Re-run the structural ZIP/Mach-O verifier on the downloaded archive,
        # without signing or extracting any user files.
        package_spec = importlib.util.spec_from_file_location("package_ipa", Path(__file__).with_name("package-ipa.py"))
        package_ipa = importlib.util.module_from_spec(package_spec)
        package_spec.loader.exec_module(package_ipa)
        name = f"NexusIDE-{identity['variant']}-{identity['commit'][:12]}-unsigned.ipa"
        facts = package_ipa.verify(args.package / name, identity["variant"])
        build, _ = ci.read_json(args.package / "build.json")
        if any(not ci.equal(build.get(key), value) for key, value in facts.items()):
            raise ValueError("Downloaded IPA structure differs from build metadata")
        print("Verified exact release candidate and untouched device acceptance template")
    else:
        if args.evidence is None or args.output is None:
            parser.error("--evidence and --output required")
        record, _ = ci.read_json(args.record)
        result = validate(identity, record, args.evidence)
        with args.output.open("x") as f:
            json.dump(result, f, indent=2, sort_keys=True)
            f.write("\n")
        print("Owner-attested review complete" if result["operatorReviewComplete"] else "Device checks pending")
        raise SystemExit(0 if result["operatorReviewComplete"] else 1)
