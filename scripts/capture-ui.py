#!/usr/bin/env python3
"""Capture the native debug UI with simulated events; no serial ports are opened.

Requires an active desktop session. Eframe reads its own framebuffer; no desktop
or other application's pixels are captured. Uses only the Python standard library.
"""
import argparse
import os
import pathlib
import struct
import subprocess
import tempfile
import zlib

ROOT = pathlib.Path(__file__).resolve().parents[1]


def encode_png(ppm, destination):
    magic, dimensions, maximum, pixels = ppm.read_bytes().split(b'\n', 3)
    assert magic == b'P6' and maximum == b'255'
    width, height = map(int, dimensions.split())
    assert len(pixels) == width * height * 3

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))

    rows = b''.join(b'\0' + pixels[y*width*3:(y+1)*width*3] for y in range(height))
    destination.write_bytes(b'\x89PNG\r\n\x1a\n'
        + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
        + chunk(b'IDAT', zlib.compress(rows, 9)) + chunk(b'IEND', b''))
    return width, height


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--output', type=pathlib.Path, default=ROOT / 'docs/images')
    args = parser.parse_args()
    if not args.skip_build:
        subprocess.run(['cargo', 'build', '--locked'], cwd=ROOT, check=True)
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='su03t-screenshots-') as directory:
        for language in ('zh', 'en'):
            for scene in ('transfer', 'power', 'retry'):
                stem = f'{scene}-{language}'
                ppm = pathlib.Path(directory) / f'{stem}.ppm'
                env = os.environ.copy()
                env.update(SU03T_UI_PREVIEW=scene, SU03T_UI_LANGUAGE=language, SU03T_UI_CAPTURE=str(ppm))
                result = subprocess.run([str(ROOT / 'target/debug/su03t-update')], cwd=ROOT,
                                        env=env, capture_output=True, timeout=20)
                if result.returncode or not ppm.exists():
                    raise RuntimeError(f'{stem}: {result.stderr.decode(errors="replace")}')
                destination = args.output / f'{stem}.png'
                width, height = encode_png(ppm, destination)
                print(f'{destination.relative_to(ROOT)} ({width}x{height})', flush=True)


if __name__ == '__main__':
    main()
