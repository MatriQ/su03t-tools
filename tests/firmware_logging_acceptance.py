#!/usr/bin/env python3
"""File-only known-profile logging/audio acceptance; never opens any device."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('firmware', nargs='+', type=Path)
    args = parser.parse_args()
    binary = str(args.binary.resolve())

    def cli(*argv, ok=True):
        result = subprocess.run([binary, *map(str, argv)], capture_output=True, timeout=40)
        assert (result.returncode == 0) == ok, f'Unexpected exit: {argv[0]}'
        return json.loads(result.stdout) if ok else None

    def runtime(report):
        return next(s for s in report['settings'] if s['kind'] == 'runtime')

    def inspect(path):
        return cli('firmware-inspect', '--firmware', path, '--json')

    cases = []
    with tempfile.TemporaryDirectory(prefix='su03t-log-acceptance-') as temp:
        temp = Path(temp)
        source_wav = temp / 'synthetic.wav'
        with wave.open(str(source_wav), 'wb') as audio:
            audio.setparams((1, 2, 16000, 4000, 'NONE', 'not compressed'))
            audio.writeframes(b''.join(struct.pack('<h', round(math.sin(i * 0.1728) * 6000)) for i in range(4000)))
        for index, source in enumerate(args.firmware):
            original = source.read_bytes()
            before = inspect(source)
            layout = before['audio']
            assert layout
            skip = 65536 if layout['full_image'] else 0
            baseline = original[skip:]
            setting = runtime(before['logging'])
            assert setting['editable'] and isinstance(setting['enabled'], bool)
            offset = setting['source_offset'] - skip
            assert baseline[offset] == (ord('1') if setting['enabled'] else ord('0'))
            config = next(c for c in before['configurations'] if c['kind'] == 'runtime')
            assert config['value']['log']['enable'] == int(setting['enabled'])
            assert cli('firmware-logging', '--firmware', source) == before['logging']
            for state in (True, False):
                target = temp / f'{index}-{state}.bin'
                manifest = cli('firmware-logging', '--firmware', source, '--runtime', 'on' if state else 'off', '--output', target)
                actual = target.read_bytes()
                expected = bytearray(baseline)
                expected[offset] = ord('1') if state else ord('0')
                assert actual == expected
                parsed_target = inspect(target)
                assert runtime(parsed_target['logging'])['enabled'] is state
                parsed_config = next(c for c in parsed_target['configurations'] if c['kind'] == 'runtime')
                assert parsed_config['value']['log']['enable'] == int(state)
                assert manifest['schema_version'] == 2
                assert manifest['device_verified'] is False and manifest['experimental'] is True
                assert len(manifest['logging_changes']) == int(state != setting['enabled'])
                declared = {i for r in manifest['changed_ranges'] for i in range(r['update_offset'], r['update_offset'] + r['length'])}
                assert declared == ({offset} if state != setting['enabled'] else set())
                cli('firmware-logging', '--firmware', source, '--runtime', 'on', '--output', target, ok=False)
            # Both edits compose through reopening, in either order.
            first = temp / f'{index}-True.bin'
            combined = temp / f'{index}-combined.bin'
            reply = max(layout['replies'], key=lambda r: r['capacity'])
            cli('firmware-replace', '--firmware', first, '--reply', reply['id'], '--wav', source_wav, '--output', combined)
            parsed = inspect(combined)
            assert runtime(parsed['logging'])['enabled'] is True
            assert parsed['audio']['profile'] == layout['profile']
            allowed = {offset}
            allowed.update(range(reply['offset'] - skip, reply['offset'] - skip + reply['capacity']))
            allowed.update(range(reply['directory_length_offset'] - skip, reply['directory_length_offset'] - skip + 4))
            assert all(a == b or i in allowed for i, (a, b) in enumerate(zip(baseline, combined.read_bytes())))
            off = temp / f'{index}-combined-off.bin'
            cli('firmware-logging', '--firmware', combined, '--runtime', 'off', '--output', off)
            assert runtime(inspect(off)['logging'])['enabled'] is False
            # Rename/corrupt the admitted field, or change code/model; never patch based on names alone.
            for n, position in enumerate((setting['source_offset'], setting['source_offset'] - 4, skip + 0x100, layout['directory_offset'] + len(layout['replies']) * 12 + 100,
                    next(s for s in before['logging']['settings'] if s['kind'] == 'engine')['source_offset'])):
                bad = bytearray(original)
                bad[position] ^= 4
                corrupt = temp / f'{index}-corrupt-{n}.bin'
                corrupt.write_bytes(bad)
                assert not runtime(inspect(corrupt)['logging'])['editable']
                rejected = temp / f'{index}-reject-{n}.bin'
                cli('firmware-logging', '--firmware', corrupt, '--runtime', 'on', '--output', rejected, ok=False)
                assert not rejected.exists()
            assert source.read_bytes() == original
            cases.append({'profile': layout['profile'], 'full': layout['full_image'], 'original_enabled': setting['enabled'], 'source_sha256': hashlib.sha256(original).hexdigest(), 'passed': True})
    print(json.dumps({'hardware_accessed': False, 'cases': cases}, indent=2))


if __name__ == '__main__':
    main()
