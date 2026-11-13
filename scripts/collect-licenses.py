#!/usr/bin/env python3
"""Collect dependency notices for a binary package using locked Cargo metadata.

Crate archives occasionally omit their license files. In that case, read the
upstream license at the crate's recorded Git commit. Downloads are cached under
target/; no GitHub credentials or additional Python packages are required.
"""
import argparse
import concurrent.futures
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
CACHE = ROOT / 'target/third-party-license-cache'
LICENSE_NAME = re.compile(r'licen[cs]e|copying|copyright|notice|\bofl\b|\bufl\b', re.I)
UPSTREAM_NAMES = ('LICENSE', 'LICENSE.txt', 'LICENSE.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'LICENSE.chromium', 'COPYING')


def fetch(url):
    cache = CACHE / hashlib.sha256(url.encode()).hexdigest()
    if cache.exists():
        return cache.read_text()
    if cache.with_suffix('.missing').exists():
        return None
    try:
        request = urllib.request.Request(url, headers={'User-Agent': 'su03t-license-collector'})
        with urllib.request.urlopen(request, timeout=30) as response:
            content = response.read().decode('utf-8')
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        cache.with_suffix('.missing').touch()
        return None
    cache.write_text(content)
    return content


def packages(targets):
    selected = {}
    for target in targets:
        command = ['cargo', 'metadata', '--locked', '--offline', '--format-version', '1']
        if target:
            command += ['--filter-platform', target]
        metadata = json.loads(subprocess.check_output(command, cwd=ROOT))
        nodes = {node['id']: node for node in metadata['resolve']['nodes']}
        pending = [metadata['resolve']['root']]
        reachable = set()
        while pending:
            package_id = pending.pop()
            if package_id in reachable:
                continue
            reachable.add(package_id)
            for dependency in nodes[package_id]['deps']:
                if any(kind['kind'] != 'dev' for kind in dependency['dep_kinds']):
                    pending.append(dependency['pkg'])
        for package in metadata['packages']:
            if package['id'] in reachable and package['source']:
                selected[package['id']] = package
    return sorted(selected.values(), key=lambda p: (p['name'], p['version']))


def notices(package):
    directory = pathlib.Path(package['manifest_path']).parent
    files = [p for p in directory.rglob('*') if p.is_file()
             and LICENSE_NAME.search(p.name) and p.suffix not in ('.rs', '.py')]
    if package['license_file']:
        files.append(directory / package['license_file'])
    result = [(str(p.relative_to(directory)), p.read_text()) for p in sorted(set(files))]
    if result:
        return result
    vcs_path = directory / '.cargo_vcs_info.json'
    repository = package.get('repository') or ''
    match = re.match(r'https?://github.com/([^/]+/[^/]+)', repository)
    if match and vcs_path.exists():
        commit = json.loads(vcs_path.read_text())['git']['sha1']
        repository = match[1].removesuffix('.git')
        for filename in UPSTREAM_NAMES:
            url = f'https://raw.githubusercontent.com/{repository}/{commit}/{filename}'
            content = fetch(url)
            if content:
                result.append((url, content))
    if result:
        return result
    # These crates publish a license declaration but no separate license file.
    # Preserve their declared authors alongside the corresponding license text.
    declared = package.get('license')
    if declared in ('MIT', 'CC0-1.0'):
        url = f'https://spdx.org/licenses/{declared}.txt'
        content = fetch(url)
        if content:
            return [(url, content)]
    raise RuntimeError(f"No license text found for {package['name']} {package['version']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', action='append', default=[])
    parser.add_argument('--output', required=True, type=pathlib.Path)
    args = parser.parse_args()
    CACHE.mkdir(parents=True, exist_ok=True)
    dependencies = packages(args.target or [None])
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as workers:
        collected = list(workers.map(notices, dependencies))
    output = ['Third-party notices', '===================', '',
              'Collected from Cargo.lock for: ' + ', '.join(args.target or ['host platform']),
              'Includes normal and build-time dependencies; dev-only dependencies are excluded.',
              'The project MIT license does not replace the licenses below.', '']
    for package, texts in zip(dependencies, collected):
        output += ['=' * 72, f"{package['name']} {package['version']}",
                   'License: ' + (package.get('license') or 'see license file'),
                   'Authors: ' + ', '.join(package['authors']),
                   'Source: ' + (package.get('repository') or package['source']), '']
        for source, text in texts:
            output += ['--- ' + source + ' ---', text.strip(), '']
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / 'THIRD_PARTY_NOTICES.txt').write_text('\n'.join(output))
    rust = pathlib.Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip()) / 'share/doc/rust'
    copyright_file = rust / 'COPYRIGHT-library.html'
    if not copyright_file.exists():
        raise RuntimeError('Rust standard-library notices are missing; install the rust-docs component.')
    shutil.copy2(copyright_file, args.output / copyright_file.name)
    shutil.copytree(rust / 'licenses', args.output / 'licenses', dirs_exist_ok=True)
    print(f'Collected {len(dependencies)} dependency notices and Rust library licenses.')


if __name__ == '__main__':
    main()
