"""Run the actual host TCI guest and verify a forwarded container HTTP request."""
import json
import pathlib
import socket
import subprocess
import sys
import time
import urllib.request

out, qemu, kernel, initrd, disk = sys.argv[1:]
out = pathlib.Path(out)
# Avoid accidentally testing an unrelated listener.
with socket.socket() as check:
    check.bind(("127.0.0.1", 18766))
args = [qemu, "-machine", "virt,gic-version=3", "-cpu", "cortex-a57",
        "-accel", "tcg,thread=single", "-smp", "1", "-m", "1024",
        "-nodefaults", "-no-user-config", "-display", "none", "-monitor", "none",
        "-serial", "stdio", "-no-reboot", "-kernel", kernel, "-initrd", initrd,
        "-append", "root=/dev/vda rw rootwait console=ttyAMA0 init=/usr/local/sbin/nexus-vm-smoke nexus.net=1",
        "-drive", f"if=none,file={disk},format=raw,id=root",
        "-device", "virtio-blk-device,drive=root",
        "-netdev", "user,id=nexusnet,restrict=on,ipv6=off,hostfwd=tcp:127.0.0.1:18766-10.0.2.15:8080",
        "-device", "virtio-net-device,netdev=nexusnet"]
serial = out / "serial.txt"
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
deadline = time.monotonic() + 2700
http_ok = False
with serial.open("wb") as log:
    proc = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
    try:
        while proc.poll() is None and time.monotonic() < deadline:
            # Bounded tail; no assumption about buffering or timing.
            with serial.open("rb") as reader:
                reader.seek(max(0, serial.stat().st_size - 262144))
                recent = reader.read()
            if not http_ok and b"NEXUS_VM_FORWARD_READY" in recent:
                try:
                    with opener.open("http://127.0.0.1:18766/cgi-bin/probe", timeout=5) as response:
                        status, body = response.status, response.read(256)
                    if status != 200 or body != b"NEXUS_VM_HTTP_42\n":
                        raise RuntimeError("Unexpected forwarded response")
                    (out / "forward-http.json").write_text(json.dumps(
                        {"ok": True, "status": status, "body": body.decode().strip()}) + "\n")
                    http_ok = True
                except OSError:
                    pass
            time.sleep(1)
        if proc.poll() is None:
            raise TimeoutError("Full-system regression timed out")
        if proc.returncode or not http_ok:
            raise RuntimeError(f"QEMU exit={proc.returncode}, forwarded HTTP verified={http_ok}")
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        with serial.open("rb") as reader:
            reader.seek(max(0, serial.stat().st_size - 16384))
            print(reader.read().decode(errors="replace"))
