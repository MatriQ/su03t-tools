#!/usr/bin/env python3
"""Verify startup diagnostics against PTYs; never open a hardware port."""
import os
import pathlib
import pty
import select
import subprocess
import sys
import tempfile
import threading

exe = pathlib.Path(sys.argv[1]).resolve()
for payload in (b'', b'\x00\xffCC', b'\x80' * 300):
    with tempfile.TemporaryDirectory(prefix='su03t-handshake-') as directory:
        firmware = pathlib.Path(directory) / 'test_update.bin'
        firmware.write_bytes(b'test firmware')
        master, slave = pty.openpty()
        proc = subprocess.Popen(
            [str(exe), 'flash', '--port', os.ttyname(slave),
             '--firmware', str(firmware), '--wait', '1', '--virtual-port'],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        stop = threading.Event()
        drained = bytearray()
        reader = None
        try:
            assert select.select([master], [], [], 5)[0], 'missing startup trigger'
            wire = os.read(master, 4096)
            assert wire and set(wire) == {0x32}, wire
            if payload:
                os.write(master, payload)
            # macOS PTY tcdrain waits until its peer consumes outgoing bytes.
            # Keep consuming triggers, even when the simulated device is silent.
            def drain():
                while not stop.is_set():
                    if select.select([master], [], [], .05)[0]:
                        drained.extend(os.read(master, 4096))
            reader = threading.Thread(target=drain)
            reader.start()
            _, error = proc.communicate(timeout=5)
            stop.set()
            reader.join()
            wire += bytes(drained)
            text = error.decode()
            assert proc.returncode != 0 and 'waiting for CCC timed out' in text, text
            assert f'握手阶段累计接收：{len(payload)} 字节' in text, text
            assert 'PROGRESS' not in text, text
            if payload:
                expected = ' '.join(f'{byte:02X}' for byte in payload[:256])
                assert f'握手RX HEX（最多前256字节）：{expected}\n' in text, text
                assert text.count('启动RX +') == min(len(payload), 8), text
            else:
                assert '握手阶段未收到任何字节' in text, text
            while select.select([master], [], [], 0)[0]:
                wire += os.read(master, 4096)
            assert set(wire) == {0x32}, 'firmware was transmitted without a handshake'
            print(f'PASS: {len(payload)} startup bytes, bounded HEX log, no firmware sent')
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait()
            stop.set()
            if reader is not None:
                reader.join()
            os.close(master)
            os.close(slave)
