"""CI-only Chromium UI test for the pinned local OpenVSCode guest.
No user credentials, remote workspace, screenshots, or browser log export.
"""
import os
import re
import signal
from urllib.parse import urlencode
from playwright.sync_api import sync_playwright

def timeout(*_):
    raise TimeoutError("Workbench browser test timed out")

def main():
    token = os.environ.get("NEXUS_WORKBENCH_TEST_TOKEN", "")
    if token != "f" * 64:
        raise ValueError("Expected synthetic CI token")
    url = "http://127.0.0.1:18765/?" + urlencode({"tkn": token, "folder": "/root/nexus-workbench"})
    signal.signal(signal.SIGALRM, timeout)
    signal.alarm(120)
    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch(headless=True)
            context = browser.new_context(viewport={"width": 1280, "height": 900})
            # No non-loopback HTTP downloads or analytics in the UI fixture.
            context.route("**/*", lambda route: route.continue_() if route.request.url.startswith("http://127.0.0.1:18765/") else route.abort())
            page = context.new_page()
            page.set_default_timeout(25000)
            page.goto(url, wait_until="domcontentloaded")
            page.locator(".monaco-workbench").wait_for(state="visible")
            page.keyboard.press("F1")
            prompt = page.locator(".quick-input-widget input")
            prompt.wait_for(state="visible")
            prompt.fill(">Nexus: Open CI Fixture")
            page.get_by_text("Nexus: Open CI Fixture", exact=True).first.wait_for(state="visible")
            prompt.press("Enter")
            page.get_by_text("nexus-editor-fixture.txt", exact=True).first.wait_for(state="visible")
            editor = page.locator(".editor-instance .monaco-editor .view-lines").first
            editor.wait_for(state="visible")
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
            suggestion.wait_for(state="visible")
            page.keyboard.press("Enter")
            page.keyboard.press("Meta+S")
            page.wait_for_timeout(3000)
            context.close()
            browser.close()
    finally:
        signal.alarm(0)

if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Playwright exception messages may contain the URL. Never log tokens.
        raise SystemExit("Workbench browser fixture failed")
