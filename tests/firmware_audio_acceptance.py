#!/usr/bin/env python3
"""Explicit local BIN acceptance. Uses files and synthetic WAV only; never devices.

Vendor/private images are optional inputs and must never be committed. Output
contains profile names, counts and hashes only, never recovered command text.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import wave


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('firmware', nargs='+', type=Path)
    parser.add_argument('--ffmpeg', default=shutil.which('ffmpeg'))
    args = parser.parse_args()
    assert args.ffmpeg, 'FFmpeg is required for this explicit acceptance test'
    binary = str(args.binary.resolve())

    def cli(*argv, ok=True):
        result = subprocess.run([binary, *map(str, argv)], capture_output=True, timeout=40)
        assert (result.returncode == 0) == ok, f'Unexpected exit for {argv[0]}: {result.returncode}'
        return result.stdout

    def inspect(path):
        return json.loads(cli('firmware-inspect', '--firmware', path, '--json'))

    def decode(mp3):
        result = subprocess.run([args.ffmpeg, '-v', 'error', '-nostdin', '-f', 'mp3', '-i', 'pipe:0', '-ac', '1', '-ar', '16000', '-f', 's16le', 'pipe:1'], input=mp3, capture_output=True, timeout=10)
        assert result.returncode == 0 and len(result.stdout) > 0
        return result.stdout

    reports = []
    with tempfile.TemporaryDirectory(prefix='su03t-studio-acceptance-') as temp:
        temp = Path(temp)
        source = temp / 'synthetic.wav'
        samples = [round(math.sin(i * 2 * math.pi * 440 / 48000) * 12000) for i in range(24000)]
        with wave.open(str(source), 'wb') as wav:
            wav.setparams((1, 2, 48000, len(samples), 'NONE', 'not compressed'))
            wav.writeframes(struct.pack('<' + 'h' * len(samples), *samples))
        for index, path in enumerate(args.firmware):
            original = path.read_bytes()
            report = inspect(path)
            assert report['sha256'] == sha(original)
            layout = report['audio']
            assert layout and report['firmware_edit_supported']
            skip = 65536 if layout['full_image'] else 0
            for reply in layout['replies']:
                mp3 = original[reply['offset']:reply['offset'] + reply['mp3_bytes']]
                assert len(mp3) % 72 == 0
                assert len(decode(mp3)) == len(mp3) // 72 * 576 * 2
            reply = max(layout['replies'], key=lambda r: r['capacity'])
            target = temp / f'{index}_update.bin'
            argv = ['firmware-replace', '--firmware', path, '--reply', reply['id'], '--wav', source, '--output', target, '--ffmpeg', args.ffmpeg, '--start', '0.05', '--end', '0.4', '--gain', '0.7']
            manifest = json.loads(cli(*argv))
            edited = target.read_bytes()
            assert json.loads(Path(str(target) + '.manifest.json').read_text()) == manifest
            assert manifest['experimental'] and not manifest['device_verified']
            assert manifest['source_sha256'] == sha(original) and manifest['update_sha256'] == sha(edited)
            assert manifest['removed_boot_bytes'] == skip
            assert manifest['modified_reply_ids'] == [reply['id']]
            assert len(edited) == len(original) - skip
            allowed = [(reply['offset'] - skip, reply['offset'] + reply['capacity'] - skip), (reply['directory_length_offset'] - skip, reply['directory_length_offset'] + 4 - skip)]
            changes = {i for i, (a, b) in enumerate(zip(original[skip:], edited)) if a != b}
            assert changes and all(any(a <= i < b for a, b in allowed) for i in changes)
            declared = {i for r in manifest['changed_ranges'] for i in range(r['update_offset'], r['update_offset'] + r['length'])}
            assert changes == declared
            after = inspect(target)
            assert after['audio']['profile'] == layout['profile']
            assert after['commands'] == [{**c, 'span': {**c['span'], 'offset': c['span']['offset'] - skip}, 'pointer_offset': c['pointer_offset'] - skip if c['pointer_offset'] is not None else None} for c in report['commands']]
            extracted = temp / f'{index}.mp3'
            cli('firmware-extract', '--firmware', target, '--reply', reply['id'], '--output', extracted)
            decoded = decode(extracted.read_bytes())
            assert len(decoded) >= 5600 * 2
            values = struct.unpack('<' + 'h' * (len(decoded) // 2), decoded)
            assert max(map(abs, values)) > 1000
            for old in layout['replies']:
                if old['id'] != reply['id']:
                    assert edited[old['offset'] - skip:old['offset'] + old['capacity'] - skip] == original[old['offset']:old['offset'] + old['capacity']]
            # Reopen and edit the output again, keeping its fingerprint valid.
            repeated = temp / f'{index}_again_update.bin'
            again = argv.copy(); again[2] = target; again[8] = repeated
            cli(*again)
            assert inspect(repeated)['audio']['profile'] == layout['profile']
            # Collision refuses overwrite and leaves both output artifacts intact.
            cli(*argv, ok=False)
            assert target.read_bytes() == edited
            collision = temp / f'{index}_collision_update.bin'
            Path(str(collision) + '.manifest.json').write_text('keep')
            clash = argv.copy(); clash[8] = collision
            cli(*clash, ok=False)
            assert not collision.exists() and Path(str(collision) + '.manifest.json').read_text() == 'keep'
            # A changed code byte must disable the editing adapter.
            corrupt = bytearray(original); corrupt[-1] ^= 1
            damaged = temp / f'{index}_unknown.bin'; damaged.write_bytes(corrupt)
            assert inspect(damaged)['audio'] is None
            denied = argv.copy(); denied[2] = damaged; denied[8] = temp / f'{index}_denied_update.bin'
            cli(*denied, ok=False)
            assert not denied[8].exists()
            assert path.read_bytes() == original
            reports.append({'profile': layout['profile'], 'full': layout['full_image'], 'replies_decoded': len(layout['replies']), 'source_sha256': sha(original), 'update_sha256': sha(edited), 'changed_bytes': len(changes)})
    print(json.dumps({'hardware_accessed': False, 'cases': reports}, indent=2))


if __name__ == '__main__':
    main()
