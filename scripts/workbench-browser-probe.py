"""CI-only Chromium UI test for the pinned local OpenVSCode guest.
No user credentials, remote workspace, screenshots, or browser log export.
"""
import os
import re
import signal
import sys
import time
from urllib.parse import urlencode
from playwright.sync_api import TimeoutError as PlaywrightTimeout, sync_playwright

def timeout(*_):
    raise TimeoutError("Workbench browser test timed out")

def main():
    token = os.environ.get("NEXUS_WORKBENCH_TEST_TOKEN", "")
    if token != "f" * 64:
        raise ValueError("Expected synthetic CI token")
    url = "http://127.0.0.1:18765/?" + urlencode({"tkn": token, "folder": "/root/nexus-workbench"})
    signal.signal(signal.SIGALRM, timeout)
    signal.alarm(420)
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
            page.get_by_text("nexus-editor-fixture.txt", exact=True).first.wait_for(state="visible")
            editor = page.locator(".editor-instance .monaco-editor .view-lines").first
            editor.wait_for(state="visible")
            print("Workbench browser: edit and save", file=sys.stderr, flush=True)
            editor.click()
            page.keyboard.press("Meta+A")
            page.keyboard.insert_text("NEXUS_BROWSER_EDITOR_SAVED")
            page.keyboard.press("Meta+S")
            # Wait for the save event and extension-host disk write; Rust checks
            # all markers and actual guest bytes after clean shutdown.
            page.wait_for_timeout(1500)
            page.keyboard.press("Enter")
            page.keyboard.press("Control+Space")
            suggestion = page.locator(".suggest-widget").get_by_text("NEXUS_LSP_COMPLETION", exact=True).first
            print("Workbench browser: await LSP completion", file=sys.stderr, flush=True)
            suggestion.wait_for(state="visible")
            page.keyboard.press("Enter")
            page.keyboard.press("Meta+S")
            page.wait_for_timeout(3000)
            print("Workbench browser: completion accepted and save requested", file=sys.stderr, flush=True)
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
