#!/usr/bin/env python3
"""Exercise the shipped CLI with Unicode files and no physical serial device."""
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile

exe = str(Path(sys.argv[1]).resolve())


def run(*args):
    return subprocess.run([exe, *args], capture_output=True, encoding="utf-8", timeout=15)


version = run("--version")
assert version.returncode == 0, version.stderr
assert version.stdout.startswith("su03t-update "), version.stdout
help_result = run("--help")
assert help_result.returncode == 0 and "firmware-info" in help_result.stdout, help_result.stderr
with tempfile.TemporaryDirectory(prefix="su03t-cli-") as directory:
    image = Path(directory) / "中文 firmware_update.bin"
    data = b"synthetic image for file checks only\x00\xff"
    image.write_bytes(data)
    info = run("firmware-info", "--firmware", str(image))
    assert info.returncode == 0, info.stderr
    assert hashlib.sha256(data).hexdigest() in info.stdout, info.stdout
    rejected = run("flash", "--firmware", str(image), "--expected-sha256", "0" * 64, "--port", "COM0")
    assert rejected.returncode != 0 and "SHA-256 已变化" in rejected.stderr, rejected.stderr
    assert "无法打开" not in rejected.stderr, rejected.stderr
    blocked = run("monitor", "--profile", "b8", "--port", "COM0", "--send-hex", "00")
    assert blocked.returncode != 0 and "禁止发送" in blocked.stderr, blocked.stderr
print("PASS: version/help, Unicode firmware path, independent SHA-256 and refusals before port open")
