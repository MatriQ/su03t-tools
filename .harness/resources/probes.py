#!/usr/bin/env python3
"""Bounded, hardware-free tool, CLI, GitHub-access and CI observations."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
REPOSITORY = 'MatriQ/su03t-tools'

def run(argv, timeout=10):
    return subprocess.run(argv, cwd=ROOT, text=True, capture_output=True,
                          timeout=timeout, check=True).stdout.strip()

def tools():
    versions = {name: run(argv) for name, argv in {
        'git':['git','--version'], 'node':['node','--version'],
        'cargo':['cargo','--version'], 'rustc':['rustc','--version'],
        'python':['python3','--version'], 'gh':['gh','--version'],
    }.items()}
    node = int(versions['node'].removeprefix('v').split('.')[0])
    rust = tuple(map(int, re.search(r'(\d+)\.(\d+)', versions['rustc']).groups()))
    return {'target':{'environment':'local','identity':'development-toolchain'},
            'accepted':node >= 24 and rust >= (1,95), 'versions':versions}

def access():
    info = json.loads(run(['gh','api',f'repos/{REPOSITORY}']))
    return {'target':{'environment':'github','identity':info['full_name']},
            'accepted':info['full_name']==REPOSITORY and info['default_branch']=='main'}

def cli():
    data = json.loads(run(['cargo','metadata','--locked','--no-deps','--format-version','1']))
    package = next(p for p in data['packages'] if Path(p['manifest_path'])==ROOT/'Cargo.toml')
    target = Path(os.environ.get('CARGO_TARGET_DIR',ROOT/'target'))
    if not target.is_absolute():
        target = ROOT/target
    binary = target/'debug'/('su03t-update.exe' if os.name=='nt' else 'su03t-update')
    version = run([str(binary),'--version'])
    with tempfile.TemporaryDirectory(prefix='su03t-harness-smoke-') as directory:
        image = Path(directory)/'smoke_update.bin'
        payload = b'harness smoke\n'
        image.write_bytes(payload)
        output = run([str(binary),'firmware-info','--firmware',str(image)])
    observed = version.removeprefix('su03t-update ')
    return {'target':{'environment':'local','identity':'su03t-update-cli'},
            'artifact':{'kind':'package_version','value':observed},
            'accepted':version==f"su03t-update {package['version']}" and hashlib.sha256(payload).hexdigest() in output}

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('check',choices=('tools','access','cli'))
    args=parser.parse_args()
    try:
        result=globals()[args.check]()
    except (OSError,ValueError,KeyError,StopIteration,subprocess.SubprocessError) as error:
        print(json.dumps({'accepted':False,'error':type(error).__name__}))
        raise SystemExit(1)
    print(json.dumps(result,sort_keys=True))
    raise SystemExit(0 if result['accepted'] else 1)
