#!/usr/bin/env python3
"""Run the real CLI against a macOS PTY, never an attached hardware device."""
import binascii
import os
import pathlib
import pty
import select
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
EXE = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'target/debug/su03t-update'

def exercise(cancel=False, skip=False, drop_ack=False):
    # More than 256 packets, including a short final packet.
    data = bytes(range(256)) * 1025 + b'final-block'
    with tempfile.TemporaryDirectory(prefix='su03t-simulator-') as directory:
        path = pathlib.Path(directory) / 'simulated_update.bin'
        path.write_bytes(data)
        master, slave = pty.openpty()
        port = os.ttyname(slave)
        proc = subprocess.Popen([str(EXE), 'flash', '--port', port, '--firmware', str(path),
                                 '--wait', '3', '--timeout-ms', '200', '--retries', '2', '--virtual-port'],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        def read(n):
            out = b''
            deadline = time.monotonic() + 10
            while len(out) < n:
                if proc.poll() is not None:
                    out_log, err_log = proc.communicate()
                    raise AssertionError(f'CLI exited {proc.returncode}: {err_log.decode(errors="replace")}')
                if time.monotonic() >= deadline:
                    raise AssertionError(f'timeout receiving {n} bytes; got {len(out)}')
                ready, _, _ = select.select([master], [], [], .1)
                if ready:
                    out += os.read(master, n-len(out))
            return out
        try:
            # The vendor startup trigger must be sent before simulated power-on.
            assert read(1) == b'2'
            os.write(master, b'boot\r\nCC')
            time.sleep(.005)
            os.write(master, b'C' if not skip else b'\r\nNNN')
            if skip:
                out, err = proc.communicate(timeout=5)
                assert proc.returncode != 0 and b'NNN' in err, err
                return
            if cancel:
                # Consume queued startup ASCII 2 bytes, then exactly one first packet.
                first = read(1)
                while first == b'2': first = read(1)
                assert first == b'\x02'
                read(1028)
                os.write(master, b'\x18')
                out, err = proc.communicate(timeout=5)
                assert proc.returncode != 0 and b'CAN' in err, err
                return
            total = (len(data)+1023)//1024
            for i in range(total):
                first = read(1)
                if i == 0:
                    while first == b'2': first = read(1)
                p = first + read(1028)
                assert p[:3] == bytes([2, (i+1)&255, 255-((i+1)&255)]), (i,p[:3])
                payload = data[i*1024:(i+1)*1024].ljust(1024, b'\x1a')
                assert p[3:1027] == payload, i
                # Independent Python standard-library CRC reference.
                assert p[1027:] == binascii.crc_hqx(payload, 0).to_bytes(2,'big'), i
                if i == 2:
                    if not drop_ack: os.write(master,b'\x15')
                    assert read(1029) == p, 'retry must reuse exact bytes/sequence'
                os.write(master,b'\x06')
            assert read(1) == b'\x04'
            os.write(master,b'\x15')
            assert read(1) == b'\x04'
            os.write(master,b'\x06')
            out, err = proc.communicate(timeout=5)
            assert proc.returncode == 0, err.decode(errors='replace')
            assert f'PROGRESS {len(data)} {len(data)}'.encode() in err
            print(f'PASS: {total} packets, sequence wrap, CRC, padding, {"timeout" if drop_ack else "NAK"} retry, EOT retry')
        finally:
            if proc.poll() is None: proc.kill(); proc.wait()
            os.close(master); os.close(slave)

exercise()
exercise(drop_ack=True)
exercise(cancel=True)
print('PASS: receiver CAN is an error')
exercise(skip=True)
print('PASS: NNN is explicit skip/error, never a false success')
