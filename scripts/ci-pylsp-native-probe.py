"""CI-only native ARM64 control for the actual pinned server and snapshot wire."""
import json
import os
import select
import subprocess
import time

URI = "file:///tmp/nexus-lsp.py"
TEXT = "x = missing_nexus_symbol"

def probe():
    p = subprocess.Popen(
        ["/usr/bin/python3", "-u", "-m", "pylsp", "-v"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, cwd="/tmp",
        env=dict(os.environ, PYTHONFAULTHANDLER="1"),
    )
    buffer = bytearray()
    deadline = time.monotonic() + 90
    def send(method, params=None, id_=None):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        if id_ is not None:
            msg["id"] = id_
        body = json.dumps(msg, separators=(",", ":")).encode()
        p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        p.stdin.flush()
    def receive():
        while True:
            idx = buffer.find(b"\r\n\r\n")
            if idx >= 0:
                header = buffer[:idx].split(b"\r\n")
                size = int(next(x.split(b":",1)[1] for x in header if x.lower().startswith(b"content-length:")))
                if not 0 < size <= 32768:
                    raise ValueError("LSP response size")
                if len(buffer) >= idx+4+size:
                    result = json.loads(buffer[idx+4:idx+4+size])
                    del buffer[:idx+4+size]
                    return result
            remaining = deadline-time.monotonic()
            if remaining <= 0 or not select.select([p.stdout], [], [], remaining)[0]:
                raise TimeoutError("Native control LSP response")
            data = os.read(p.stdout.fileno(), 4096)
            if not data:
                raise RuntimeError("Native control LSP EOF")
            buffer.extend(data)
            if len(buffer)>65536:
                raise ValueError("Native control LSP buffer limit")
    def until(predicate):
        for _ in range(128):
            frame = receive()
            if frame.get("jsonrpc") != "2.0" or ("id" in frame and "method" in frame):
                raise ValueError("Unexpected LSP frame")
            if predicate(frame):
                return frame
        raise ValueError("Native control LSP frame limit")
    try:
        send("initialize", {"processId": None, "rootUri": "file:///tmp", "workspaceFolders": None,
             "capabilities": {"general": {"positionEncodings": ["utf-16"]}, "textDocument": {
             "publishDiagnostics": {"versionSupport": True}, "completion": {"completionItem": {"snippetSupport": False}}}},
             "clientInfo": {"name": "nexus-ios", "version": "0.8.0"}}, 1)
        assert "capabilities" in until(lambda f: f.get("id")==1)["result"]
        print("Native control: initialize verified", flush=True)
        send("initialized", {})
        send("textDocument/didOpen", {"textDocument": {"uri": URI, "languageId": "python", "version": 1, "text": TEXT}})
        frame = until(lambda f: f.get("method")=="textDocument/publishDiagnostics")
        assert frame["params"]["uri"]==URI
        assert any("missing_nexus_symbol" in d["message"] and d["range"]["start"]=={"line":0,"character":4}
                   for d in frame["params"]["diagnostics"]), frame
        print("Native control: actual undefined-symbol diagnostic verified", flush=True)
        send("textDocument/didClose", {"textDocument":{"uri":URI}})
        send("shutdown", id_=2)
        assert until(lambda f:f.get("id")==2)["result"] is None
        send("exit")
        assert p.wait(timeout=15)==0
        print("Native control: clean LSP exit", flush=True)
    finally:
        if p.poll() is None:
            p.kill()
            p.wait()

if __name__ == "__main__":
    probe()
