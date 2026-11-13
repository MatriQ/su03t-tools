#!/usr/bin/env python3
"""Install a real build archive into a temporary prefix through mocked downloads."""
import argparse
import hashlib
import pathlib
import platform
import shutil
import subprocess
from test_installer import InstallerTests

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--archive', type=pathlib.Path, required=True)
parser.add_argument('--version', required=True)
parser.add_argument('--platform', required=True)
args = parser.parse_args()
case = InstallerTests()
case.setUp()
try:
    tag = 'v' + args.version
    directory = case.fixtures / tag
    directory.mkdir()
    package = directory / args.archive.name
    shutil.copy2(args.archive, package)
    (directory / f'SU03T-Tools-{tag}-SHA256SUMS.txt').write_text(
        f'{hashlib.sha256(package.read_bytes()).hexdigest()}  {package.name}\n')
    case.env['SU03T_TEST_OS'] = 'Darwin' if args.platform == 'macOS-universal' else 'Linux'
    case.env['SU03T_TEST_ARCH'] = platform.machine()
    case.install(tag)
    binary = case.prefix / ('SU03T Update.app/Contents/MacOS/su03t-update'
                            if args.platform == 'macOS-universal' else 'bin/su03t-update')
    actual = subprocess.check_output([str(binary), '--version'], text=True).strip()
    if actual != f'su03t-update {args.version}':
        raise RuntimeError(f'Installed version mismatch: {actual}')
    print(f'PASS: installed the real {args.platform} package into an isolated prefix.')
finally:
    case.doCleanups()
