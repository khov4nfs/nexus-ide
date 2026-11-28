"""CI-only Chromium UI test for the pinned local OpenVSCode guest.
No user credentials, remote workspace, screenshots, or browser log export.
"""
import json
import os
import re
import signal
import sys
import time
from pathlib import Path
from urllib.parse import urlencode
from playwright.sync_api import TimeoutError as PlaywrightTimeout, sync_playwright

def timeout(*_):
    raise TimeoutError("Workbench browser test timed out")

def wait_for_guest_save(page, root, expected, phase, seconds=45):
    # Observe only fixed synthetic guest files. Do not create markers or
    # write workspace data from the host/browser harness.
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        checks = {}
        for name, value in expected.items():
            try:
                checks[name] = (root / name).read_bytes() == value
            except OSError:
                checks[name] = False
        if all(checks.values()):
            print(f"Workbench browser: {phase} confirmed on guest disk",
                  file=sys.stderr, flush=True)
            return
        page.wait_for_timeout(250)
    # No file contents, token or raw Playwright exception in diagnostics.
    print(f"Workbench browser: {phase} not persisted: {checks}",
          file=sys.stderr, flush=True)
    raise TimeoutError("Guest save acknowledgement missing")

def save_from_palette(page):
    # Invoke the real Workbench save command, not a fixture-side write or
    # a browser shortcut which may be intercepted by Chromium.
    page.keyboard.press("F1")
    widget = page.locator(".quick-input-widget:visible")
    prompt = widget.locator("input:visible").first
    prompt.wait_for(state="visible", timeout=10000)
    prompt.fill(">File: Save", timeout=5000)
    option = widget.get_by_role("option", name=re.compile(r"^File: Save(?:,|$)", re.I)).first
    option.wait_for(state="visible", timeout=15000)
    option.click(timeout=5000)
    widget.first.wait_for(state="hidden", timeout=10000)

def run_command(page, title):
    page.keyboard.press("F1")
    widget = page.locator(".quick-input-widget:visible")
    prompt = widget.locator("input:visible").first
    prompt.wait_for(state="visible", timeout=15000)
    prompt.fill(">" + title)
    widget.get_by_role("option", name=re.compile("^" + re.escape(title) + r"(?:,|$)", re.I)).first.click(timeout=30000)
    widget.first.wait_for(state="hidden", timeout=15000)

def exercise_c_family(page, root, language):
    cpp = language == "cpp"
    filename = "nexus-real-lsp." + language
    label = "nexus_value" if cpp else "nexus_c_value"
    initial = ("int broken = missing_nexus_cpp;\nstruct NexusCompletion { int nexus_value; };\nvoid probe() {\n    NexusCompletion item;\n    item"
               if cpp else "int broken = missing_nexus_c;\nstruct NexusC { int nexus_c_value; };\nvoid probe_c(void) {\n    struct NexusC item;\n    item")
    run_command(page, "Nexus: Open C++ CI Fixture" if cpp else "Nexus: Open C CI Fixture")
    page.get_by_text(filename, exact=True).first.wait_for(state="visible")
    editor = page.locator(".editor-instance .monaco-editor .view-lines:visible").first
    editor.wait_for(state="visible")
    editor.click()
    page.keyboard.press("Meta+A")
    page.keyboard.insert_text(initial)
    save_from_palette(page)
    wait_for_guest_save(page, root, {filename: initial.encode(),
                        "nexus-"+language+"-diagnostics.marker": b"clangd-18.1.3"}, language+" diagnostics", seconds=150)
    page.locator(".monaco-editor .squiggly-error, .monaco-editor .squiggly-warning").first.wait_for(state="visible", timeout=30000)
    editor.click()
    page.keyboard.press("Meta+End")
    page.keyboard.press("End")
    page.keyboard.insert_text(".")
    page.keyboard.press("Control+Space")
    suggestion = page.locator(".suggest-widget").get_by_text(label, exact=True).first
    suggestion.wait_for(state="visible", timeout=150000)
    page.keyboard.press("Enter")
    save_from_palette(page)
    wait_for_guest_save(page, root, {filename: (initial+"."+label).encode(),
                        "nexus-"+language+"-completion.marker": b"clangd-18.1.3",
                        "nexus-"+language+"-saved.marker": b"saved"}, language+" completion", seconds=90)
    print("Workbench browser: actual "+language+" diagnostic and completion persisted", file=sys.stderr, flush=True)

def wait_for_persistent_shutdown(page, root):
    run_command(page, "Nexus: Stop Language Servers")
    deadline = time.monotonic()+45
    while time.monotonic()<deadline:
        try:
            evidence=json.loads((root/"nexus-persistent-lsp.json").read_bytes())
        except (OSError, ValueError):
            page.wait_for_timeout(250)
            continue
        assert evidence["ok"] is True and evidence["projectAcceptance"] is True
        assert evidence["languages"] == ["python","c","cpp"]
        for key in ("python","clangd"):
            server=evidence[key]
            assert server["launches"] == 1 and server["initializes"] == 1
            assert server["shutdownClean"] is True and server["restarted"] is False
            assert server["opens"] == server["closes"]
        assert evidence["python"]["snapshotRefreshes"] >= 1
        assert evidence["clangd"]["peakDocuments"] >= 2 and evidence["clangd"]["changes"] >= 2
        return
    raise TimeoutError("Persistent LSP shutdown evidence unavailable")

def main():
    token = os.environ.get("NEXUS_WORKBENCH_TEST_TOKEN", "")
    if token != "f" * 64:
        raise ValueError("Expected synthetic CI token")
    root = Path(os.environ["NEXUS_WORKBENCH_ROOTFS"]) / "root" / "nexus-workbench"
    url = "http://127.0.0.1:18765/?" + urlencode({"tkn": token, "folder": "/root/nexus-workbench"})
    signal.signal(signal.SIGALRM, timeout)
    real = os.environ.get("NEXUS_WORKBENCH_REAL_LSP") == "1"
    filename = "nexus-real-lsp.py" if real else "nexus-editor-fixture.txt"
    initial = "missing_nexus_symbol\nclass NexusCompletion:\n    nexus_value = 42\nitem = NexusCompletion()\nitem" if real else "NEXUS_BROWSER_EDITOR_SAVED"
    label = "nexus_value" if real else "NEXUS_LSP_COMPLETION"
    final = initial + ("." if real else "\n") + label
    signal.alarm(1500 if real else 420)
    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch(headless=True)
            context = browser.new_context(viewport={"width": 1280, "height": 900})
            # No non-loopback HTTP downloads or analytics in the UI fixture.
            context.route("**/*", lambda route: route.continue_() if route.request.url.startswith("http://127.0.0.1:18765/") else route.abort())
            page = context.new_page()
            page.set_default_timeout(60000)
            print("Workbench browser: navigate", file=sys.stderr, flush=True)
            page.goto(url, wait_until="domcontentloaded")
            page.locator(".monaco-workbench").wait_for(state="visible")
            # The shell can render before the remote extension registry is
            # ready. Reopen the palette until the real installed command exists.
            # Only the synthetic local fixture workspace may be trusted here.
            print("Workbench browser: await remote command", file=sys.stderr, flush=True)
            deadline = time.monotonic() + 240
            attempt = 0
            # Quick Access computes its command picks when opened. A palette
            # opened before remote registration can retain a stale command set.
            widget = page.locator(".quick-input-widget:visible")
            command_name = re.compile(r"^Nexus: Open CI Fixture(?:,|$)", re.I)
            while time.monotonic() < deadline:
                attempt += 1
                trust = page.get_by_role("button", name=re.compile(r"^Yes, I trust the authors", re.I))
                if trust.first.is_visible():
                    trust.first.click(timeout=5000)
                prompt = widget.locator("input:visible").first
                phase = "refresh palette"
                try:
                    if widget.first.is_visible():
                        page.keyboard.press("Escape")
                        widget.first.wait_for(state="hidden", timeout=5000)
                    page.keyboard.press("F1")
                    phase = "palette input"
                    prompt.wait_for(state="visible", timeout=5000)
                    prompt.fill(">Nexus: Open CI Fixture", timeout=5000)
                    phase = "command option"
                    # Monaco highlights individual substrings and may append
                    # description/keybinding text. Match the accessible option
                    # label instead of an exact text node, then click that row.
                    command = widget.get_by_role("option", name=command_name).first
                    command.wait_for(state="visible", timeout=10000)
                    phase = "invoke command"
                    command.click(timeout=5000)
                except PlaywrightTimeout:
                    if phase == "invoke command":
                        raise
                    print(f"Workbench browser: attempt {attempt}, phase={phase}, "
                          f"options={widget.get_by_role('option').count()}",
                          file=sys.stderr, flush=True)
                    continue
                print(f"Workbench browser: invoked installed command on attempt {attempt}",
                      file=sys.stderr, flush=True)
                break
            else:
                raise TimeoutError("Installed fixture command unavailable")
            print("Workbench browser: await fixture editor", file=sys.stderr, flush=True)
            page.get_by_text(filename, exact=True).first.wait_for(state="visible")
            editor = page.locator(".editor-instance .monaco-editor .view-lines").first
            editor.wait_for(state="visible")
            print("Workbench browser: edit and save", file=sys.stderr, flush=True)
            editor.click()
            page.keyboard.press("Meta+A")
            page.keyboard.insert_text(initial)
            save_from_palette(page)
            expected = {
                "nexus-extension-activated.marker": b"activated",
                "nexus-editor-saved.marker": b"saved",
                filename: initial.encode(),
            }
            if real:
                expected["nexus-real-diagnostics.marker"] = b"pylsp-1.10.0"
            wait_for_guest_save(page, root, expected, "initial editor save", seconds=150 if real else 45)
            if real:
                page.locator(".monaco-editor .squiggly-error, .monaco-editor .squiggly-warning").first.wait_for(state="visible", timeout=30000)
                print("Workbench browser: real diagnostic rendered", file=sys.stderr, flush=True)
            editor.click()
            page.keyboard.press("Meta+End")
            page.keyboard.press("End")
            if real:
                page.keyboard.insert_text(".")
            else:
                page.keyboard.press("Enter")
            page.keyboard.press("Control+Space")
            suggestion = page.locator(".suggest-widget").get_by_text(label, exact=True).first
            print("Workbench browser: await LSP completion", file=sys.stderr, flush=True)
            suggestion.wait_for(state="visible", timeout=150000 if real else 60000)
            page.keyboard.press("Enter")
            save_from_palette(page)
            expected = {
                "nexus-editor-saved.marker": b"saved",
                "nexus-lsp-transport.marker": b"completed",
                "nexus-lsp-saved.marker": b"saved",
                filename: final.encode(),
            }
            if real:
                expected["nexus-real-completion.marker"] = b"pylsp-1.10.0"
            wait_for_guest_save(page, root, expected, "LSP completion save", seconds=90 if real else 45)
            print("Workbench browser: completion accepted and persisted", file=sys.stderr, flush=True)
            if real:
                exercise_c_family(page, root, "cpp")
                exercise_c_family(page, root, "c")
                run_command(page, "Nexus: Run Synthetic Cross-file Acceptance")
                wait_for_guest_save(page, root, {"nexus-project-lsp.marker": b"definition-references-rename",
                    "nexus-project-acceptance/shared.hpp": b"inline int nexus_renamed(int x) { return x + 1; }\n",
                    "nexus-project-acceptance/main.cpp": b'#include "shared.hpp"\nint nexus_entry() { return nexus_renamed(1); }\n'}, "real cross-file rename persisted", seconds=180)
                wait_for_persistent_shutdown(page, root)
            context.close()
            browser.close()
    finally:
        signal.alarm(0)

if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        # Playwright exception messages may contain the URL. Never log tokens.
        raise SystemExit("Workbench browser fixture failed (" + type(exc).__name__ + ")")
