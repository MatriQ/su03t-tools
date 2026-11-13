#!/usr/bin/env python3
"""Exercise the real CLI through PTYs on macOS/Linux, without touching hardware."""
import hashlib
import os
import pathlib
import pty
import select
import subprocess
import sys
import tempfile
import threading
import time

exe = pathlib.Path(sys.argv[1]).resolve()
virtual = ['--virtual-port'] if sys.platform == 'darwin' else []

with tempfile.TemporaryDirectory(prefix='su03t-acceptance-') as directory:
    root = pathlib.Path(directory)
    image = root / 'test_update.bin'
    image.write_bytes(b'unchanged image')
    info = subprocess.run([str(exe), 'firmware-info', '--firmware', str(image)], capture_output=True, timeout=5)
    assert info.returncode == 0, info.stderr
    assert hashlib.sha256(image.read_bytes()).hexdigest().encode() in info.stdout
    mismatch = subprocess.run([str(exe), 'flash', '--firmware', str(image), '--expected-sha256', '0' * 64,
                               '--port', '/dev/su03t-test-must-not-open'], capture_output=True, timeout=5)
    assert mismatch.returncode != 0 and 'SHA-256 已变化' in mismatch.stderr.decode(), mismatch.stderr
    assert '无法打开' not in mismatch.stderr.decode(), mismatch.stderr
    blocked = subprocess.run([str(exe), 'monitor', '--profile', 'b8', '--port', '/dev/su03t-test-must-not-open',
                              '--send-hex', '0A'], capture_output=True, timeout=5)
    assert blocked.returncode != 0 and '禁止发送' in blocked.stderr.decode(), blocked.stderr
    print('PASS: exact SHA-256; changed firmware rejected before serial open; B8 send rejected')

    def exercise(command, response, expected_tx=b'', profile=None, trigger=False):
        master, slave = pty.openpty()
        prefix = root / f'{command}-{profile or trigger}'
        error_path = root / f'{command}-{profile or trigger}.stderr'
        args = [str(exe), command, '--port', os.ttyname(slave), *virtual]
        if command == 'probe':
            args += ['--wait', '2'] + (['--trigger'] if trigger else [])
        else:
            args += ['--profile', profile, '--duration', '0.5', '--record', str(prefix)]
            if expected_tx:
                args += ['--send-hex', expected_tx.hex()]
        stop = threading.Event()
        wire = bytearray()
        def drain():
            while not stop.is_set():
                if select.select([master], [], [], .02)[0]:
                    wire.extend(os.read(master, 4096))
        reader = threading.Thread(target=drain)
        with error_path.open('wb') as errors:
            proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=errors)
            reader.start()
            try:
                deadline = time.monotonic() + 5
                ready = '串口已打开：' if command == 'probe' else 'SYS 已连接'
                while ready not in error_path.read_text():
                    assert proc.poll() is None, error_path.read_text()
                    assert time.monotonic() < deadline, error_path.read_text()
                    time.sleep(.01)
                os.write(master, response)
                out, _ = proc.communicate(timeout=5)
                assert proc.returncode == 0, error_path.read_text()
                stop.set()
                reader.join()
                if command == 'probe':
                    assert '已识别CCC' in error_path.read_text(), error_path.read_text()
                    assert not wire or set(wire) == {0x32}, wire
                    if trigger:
                        assert wire, 'trigger not sent'
                    else:
                        assert not wire, wire
                else:
                    assert wire == expected_tx, wire
                    assert pathlib.Path(str(prefix) + '.rx.bin').read_bytes() == response
                    text = pathlib.Path(str(prefix) + '.log').read_text()
                    assert 'SYS' in text and 'RX HEX=' in text, text
                    assert f'RX={len(response)} B'.encode() in error_path.read_bytes(), error_path.read_text()
                    assert b'RX HEX=' in out, out
                print(f'PASS: {command} {profile or ("trigger" if trigger else "passive")}, binary fidelity, no unexpected writes')
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait()
                stop.set()
                reader.join()
                os.close(master)
                os.close(slave)

    exercise('probe', b'boot\r\nCCC', trigger=False)
    exercise('probe', b'boot\r\nCCC', trigger=True)
    exercise('monitor', b'\x00\x01\x80\xff\x02\x03\x04', profile='car')
    exercise('monitor', b'log\x00\x80\xff\r\n', profile='b8')
    exercise('monitor', b'\x00\xff', expected_tx=b'\x55\xaa\x00\x80\xff', profile='custom')
