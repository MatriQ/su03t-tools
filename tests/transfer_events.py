#!/usr/bin/env python3
"""Verify real CLI worker events against PTYs, never attached hardware."""
import binascii
import json
import os
import pathlib
import pty
import select
import subprocess
import sys
import tempfile
import time

EXE = pathlib.Path(sys.argv[1]).resolve()
VIRTUAL = ['--virtual-port'] if sys.platform == 'darwin' else []
PREFIX = 'SU03T_EVENT '

with tempfile.TemporaryDirectory(prefix='su03t-events-') as directory:
    root = pathlib.Path(directory)
    firmware = root / 'test_update.bin'
    payload = b'event regression\x00\x80\xff'
    firmware.write_bytes(payload)

    def decode(path):
        raw = path.read_text()
        lines = raw.splitlines()
        if raw and not raw.endswith("\n"):
            lines = lines[:-1]
        assert all(line.startswith(PREFIX) for line in lines), lines
        return [json.loads(line[len(PREFIX):]) for line in lines]

    def exercise(mode):
        master, slave = pty.openpty()
        path = root / (mode + '.events')
        command = 'probe' if mode == 'probe' else 'flash'
        args = [str(EXE), command, '--events', '--port', os.ttyname(slave), '--wait', '2', *VIRTUAL]
        if command == 'flash':
            args += ['--firmware', str(firmware), '--passive', '--timeout-ms', '80', '--retries', '2']
        with path.open('wb') as output:
            proc = subprocess.Popen(args, stdout=output, stderr=subprocess.PIPE)
            try:
                until = time.monotonic() + 5
                while not any(e['event'] == 'handshake_waiting' for e in decode(path)):
                    assert proc.poll() is None and time.monotonic() < until, path.read_text()
                    time.sleep(.005)
                events = decode(path)
                names = [e['event'] for e in events]
                assert names.index('port_opened') < names.index('handshake_waiting'), names
                assert not any(e['event'] == 'writing' for e in events), events
                os.write(master, b'CCC')

                def read(length):
                    data = bytearray()
                    deadline = time.monotonic() + 5
                    while len(data) < length:
                        assert time.monotonic() < deadline, (mode, len(data), path.read_text())
                        assert proc.poll() is None, (mode, path.read_text())
                        if select.select([master], [], [], .02)[0]:
                            data.extend(os.read(master, length - len(data)))
                    return bytes(data)

                if mode == 'probe':
                    proc.communicate(timeout=3)
                    assert not select.select([master], [], [], .02)[0], 'probe transmitted data'
                else:
                    first = read(1029)
                    assert first[:3] == b'\x02\x01\xfe'
                    expected = payload.ljust(1024, b'\x1a')
                    assert first[3:1027] == expected
                    assert first[1027:] == binascii.crc_hqx(expected, 0).to_bytes(2, 'big')
                    if mode == 'no_ack':
                        assert read(1029) == first
                        assert read(1029) == first
                    else:
                        os.write(master, b'\x15')
                        assert read(1029) == first
                        os.write(master, b'\x06')
                        assert read(1) == b'\x04'
                        if mode == 'missing_final_ack':
                            assert read(1) == b'\x04'
                            assert read(1) == b'\x04'
                        else:
                            os.write(master, b'\x06')
                    proc.communicate(timeout=3)
                events = decode(path)
                names = [e['event'] for e in events]
                assert names.index('handshake_waiting') < names.index('handshake_received'), names
                if mode == 'probe':
                    assert proc.returncode == 0 and events[-1] == {'event': 'completed', 'probe': True}, events
                    assert 'writing' not in names and 'progress' not in names, events
                else:
                    assert names.index('handshake_received') < names.index('writing') < names.index('waiting_ack'), names
                    retries = [e for e in events if e['event'] == 'retry']
                    if mode == 'no_ack':
                        assert [e['retry'] for e in retries] == [1, 2], retries
                        assert all(e['reason'] == 'timeout' for e in retries), retries
                        assert not any(e['event'] == 'progress' and e['done'] > 0 for e in events)
                    else:
                        assert retries[0]['reason'] == 'nak', retries
                    if mode in ('no_ack', 'missing_final_ack'):
                        assert proc.returncode != 0 and events[-1]['event'] == 'failed', events
                        assert events[-1]['code'] == 'ack_timeout', events[-1]
                        assert 'completed' not in names, events
                        if mode == 'missing_final_ack':
                            assert any(e['event'] == 'progress' and e['done'] == len(payload) for e in events)
                            assert [e for e in events if e['event'] == 'waiting_ack'][-1]['target']['kind'] == 'eot'
                    else:
                        assert proc.returncode == 0 and events[-1] == {'event': 'completed', 'probe': False}, events
                        assert events[-2]['event'] == 'startup_summary'
                        acknowledged = [e for e in events if e['event'] == 'acknowledged']
                        assert acknowledged[-1]['target']['kind'] == 'eot', acknowledged
                print('PASS:', mode, 'structured state ordering, retry cause, confirmed completion and wire bytes')
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait()
                os.close(master)
                os.close(slave)

    for mode in ('no_ack', 'nak_success', 'missing_final_ack', 'probe'):
        exercise(mode)

    rejected = subprocess.run([str(EXE), 'flash', '--events', '--port', '/dev/su03t-must-not-open',
                               '--firmware', str(firmware), '--expected-sha256', '0' * 64], capture_output=True, timeout=3)
    events = [json.loads(line[len(PREFIX):]) for line in rejected.stdout.decode().splitlines()]
    assert rejected.returncode != 0 and events[-1]['code'] == 'firmware_changed', events
    assert not any(e['event'] == 'port_opened' for e in events), events
    print('PASS: changed firmware reports a structured failure before opening a port')
