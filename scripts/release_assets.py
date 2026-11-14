#!/usr/bin/env python3
"""Build and verify release assets, then publish a complete GitHub Release."""
import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
PLATFORMS = ('macOS-universal', 'Linux-x86_64', 'Linux-aarch64', 'Windows-x86_64')


def platforms(value):
    return PLATFORMS if tuple(map(int, value.split('.'))) >= (0, 6, 0) else PLATFORMS[:-1]


def installers(value):
    return ['install.sh', 'install.ps1'] if 'Windows-x86_64' in platforms(value) else ['install.sh']


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def version():
    data = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=ROOT))
    package = next(p for p in data['packages'] if pathlib.Path(p['manifest_path']) == ROOT / 'Cargo.toml')
    value = package['version']
    if not re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', value):
        raise ValueError('Release packages require a stable X.Y.Z version')
    return value


def archive_name(value, platform):
    if platform not in PLATFORMS:
        raise ValueError(f'Unsupported package platform: {platform}')
    suffix = '.zip' if platform in ('macOS-universal', 'Windows-x86_64') else '.tar.gz'
    return f'SU03T-Tools-v{value}-{platform}{suffix}'


def digest(path):
    sha = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            sha.update(block)
    return sha.hexdigest()


def write_checksums(output, paths):
    names = [p.name for p in paths]
    if len(names) != len(set(names)):
        raise ValueError('Duplicate release asset names')
    output.write_text(''.join(f'{digest(p)}  {p.name}\n' for p in sorted(paths)))


def parse_checksums(text):
    result = {}
    for line in text.splitlines():
        match = re.fullmatch(r'([a-f0-9]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)', line)
        if not match or match[2] in result:
            raise ValueError('Invalid or duplicate checksum entry')
        result[match[2]] = match[1]
    if not result:
        raise ValueError('Empty checksum manifest')
    return result


def release_info(repo, tag):
    result = subprocess.run(['gh', 'api', f'repos/{repo}/releases/tags/{tag}'], capture_output=True, text=True)
    if not result.returncode:
        return json.loads(result.stdout)
    if 'HTTP 404' not in result.stderr:
        raise RuntimeError(result.stderr.strip())
    # The tag endpoint returns published releases. The list endpoint also exposes
    # drafts to writers, so an interrupted publication can find and resume one.
    result = subprocess.run(
        ['gh', 'api', '--paginate', '--slurp', f'repos/{repo}/releases?per_page=100'],
        capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    for page in json.loads(result.stdout):
        for info in page:
            if info['tag_name'] == tag:
                return info
    return None


def release_complete(repo, tag, value, allow_draft=False):
    info = release_info(repo, tag)
    if not info or (info['draft'] and not allow_draft):
        return False
    wanted = [archive_name(value, platform) for platform in platforms(value)] + installers(value)
    manifest = f'SU03T-Tools-v{value}-SHA256SUMS.txt'
    assets = {a['name']: a for a in info['assets'] if a['state'] == 'uploaded'}
    if not all(name in assets for name in wanted + [manifest]):
        return False
    with tempfile.TemporaryDirectory(prefix='su03t-published-check-') as directory:
        run(['gh', 'release', 'download', tag, '--repo', repo, '--pattern', manifest, '--dir', directory], stdout=subprocess.DEVNULL)
        checks = parse_checksums((pathlib.Path(directory) / manifest).read_text())
        if set(checks) != set(wanted):
            raise ValueError('Published checksum manifest does not match the supported release assets')
        for name in wanted:
            recorded = assets[name].get('digest')
            if recorded:
                if recorded != f'sha256:{checks[name]}':
                    raise ValueError(f'Published release checksum mismatch: {name}')
            else:
                run(['gh', 'release', 'download', tag, '--repo', repo, '--pattern', name, '--dir', directory], stdout=subprocess.DEVNULL)
                if digest(pathlib.Path(directory) / name) != checks[name]:
                    raise ValueError(f'Published release checksum mismatch: {name}')
    return True


def metadata(args):
    value = version()
    tag = args.tag or f'v{value}'
    if tag != f'v{value}':
        raise ValueError(f'Tag {tag!r} does not match Cargo.toml version v{value}')
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if args.tag:
        tagged = subprocess.check_output(['git', 'rev-parse', f'refs/tags/{tag}^{{commit}}'], cwd=ROOT, text=True).strip()
        if commit != tagged:
            raise ValueError('Checked-out commit does not match the release tag')
    complete = args.check_release and release_complete(args.repo, tag, value)
    print(f'version={value}\ntag={tag}\ncommit={commit}\nreleased={str(bool(complete)).lower()}')


def macos_archive():
    value = version()
    app = ROOT / 'dist/SU03T Update.app'
    archive = ROOT / 'dist' / archive_name(value, 'macOS-universal')
    temporary = archive.with_suffix('.new.zip')
    try:
        run(['ditto', '-c', '-k', '--keepParent', '--norsrc', '--noextattr', str(app), str(temporary)])
        temporary.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
    write_checksums(archive.parent / f'SU03T-Tools-v{value}-SHA256SUMS-macOS-universal.txt', [archive])
    print(f'Archive: {archive}')


def windows_archive(directory):
    value = version()
    for name in ('su03t-update.exe', 'su03t-tools.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.txt'):
        if not (directory / name).is_file():
            raise ValueError(f'Missing Windows package file: {name}')
    archive = ROOT / 'dist' / archive_name(value, 'Windows-x86_64')
    archive.parent.mkdir(exist_ok=True)
    temporary = archive.with_suffix('.new.zip')
    try:
        with zipfile.ZipFile(temporary, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
            for path in sorted(directory.rglob('*')):
                if path.is_symlink():
                    raise ValueError('Windows packages must not contain links')
                if path.is_file():
                    bundle.write(path, path.relative_to(directory).as_posix())
        temporary.replace(archive)
    finally:
        temporary.unlink(missing_ok=True)
    platform_checksums('Windows-x86_64')
    print(f'Archive: {archive}')


def platform_checksums(platform):
    value = version()
    archive = ROOT / 'dist' / archive_name(value, platform)
    write_checksums(archive.parent / f'SU03T-Tools-v{value}-SHA256SUMS-{platform}.txt', [archive])


def assemble(directory, value):
    paths = []
    for platform in platforms(value):
        archive = directory / archive_name(value, platform)
        part = directory / f'SU03T-Tools-v{value}-SHA256SUMS-{platform}.txt'
        checks = parse_checksums(part.read_text())
        if checks != {archive.name: digest(archive)}:
            raise ValueError(f'Package checksum mismatch: {archive.name}')
        paths.append(archive)
    for name in installers(value):
        installer = directory / name
        shutil.copy2(ROOT / name, installer)
        paths.append(installer)
    manifest = directory / f'SU03T-Tools-v{value}-SHA256SUMS.txt'
    write_checksums(manifest, paths)
    return paths + [manifest]


def publish(args):
    value = version()
    if args.tag != f'v{value}':
        raise ValueError('Release tag must match Cargo.toml')
    repo, tag = args.repo, args.tag
    paths = assemble(args.directory, value)
    if release_complete(repo, tag, value):
        print(f'Already published and verified: https://github.com/{repo}/releases/tag/{tag}')
        return
    info = release_info(repo, tag)
    with tempfile.TemporaryDirectory(prefix='su03t-release-notes-') as temporary:
        notes = pathlib.Path(temporary) / 'notes.md'
        windows_notes = ''
        if 'Windows-x86_64' in platforms(value):
            windows_notes = f'''
Windows 10/11 x64: extract the ZIP and open su03t-tools.exe; su03t-update.exe provides the CLI. / Windows 10/11 x64：解压后运行 su03t-tools.exe，命令行使用 su03t-update.exe。

```powershell
& ([scriptblock]::Create((irm https://github.com/{repo}/releases/download/{tag}/install.ps1))) -Version {tag}
```

The Windows package is unsigned. Normal Windows download protections remain in effect. / Windows 安装包未签名，保留系统下载安全检查。
'''
        notes.write_text(f'''Native SU-03T firmware updater and serial diagnostics. / SU-03T 原生升级与串口调试工具。

One-command install / 一键安装：

```sh
curl -fsSL https://github.com/{repo}/releases/download/{tag}/install.sh | bash -s -- --version {tag}
```

macOS: universal Apple Silicon / Intel app, ad-hoc signed and not notarized. First launch follows the standard macOS approval flow. / macOS 通用包使用临时签名，未公证，首次启动按系统提示确认。

Linux: x86_64 and ARM64 packages built on Ubuntu 24.04; compatible runtime libraries are required. / Linux 提供 x86_64、ARM64 包，构建环境为 Ubuntu 24.04，需匹配的运行库。
{windows_notes}
The installer verifies SHA-256 and uses user directories. Driver installation remains an explicit OS/vendor workflow. / 安装器校验 SHA-256，默认安装到用户目录；驱动按厂商和系统指引单独处理。

[中文说明](https://github.com/{repo}/blob/{tag}/README.md) · [English guide](https://github.com/{repo}/blob/{tag}/README.en.md)
''', encoding='utf-8')
        if not info:
            run(['gh', 'release', 'create', tag, '--repo', repo, '--verify-tag', '--draft',
                 '--title', f'SU-03T Tools {tag}', '--notes-file', str(notes), '--generate-notes'])
            info = release_info(repo, tag)
        if not info:
            raise RuntimeError('Created release is not visible; retry publication to resume the draft')
        assets = {a['name']: a for a in info['assets']}
        upload = []
        for path in paths:
            existing = assets.get(path.name)
            if existing and not info['draft']:
                if existing.get('digest') != f'sha256:{digest(path)}':
                    raise ValueError(f'Refusing to replace published asset {path.name}; publish a new version')
            else:
                upload.append(str(path))
        if upload:
            command = ['gh', 'release', 'upload', tag, '--repo', repo]
            if info['draft']:
                command.append('--clobber')
            run(command + upload)
        if not release_complete(repo, tag, value, allow_draft=True):
            raise RuntimeError('Release assets were incomplete; leaving the release unpublished')
        if info['draft']:
            run(['gh', 'release', 'edit', tag, '--repo', repo, '--draft=false'])
    if not release_complete(repo, tag, value):
        raise RuntimeError('Release publication was incomplete')
    print(f'Published: https://github.com/{repo}/releases/tag/{tag}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    meta = commands.add_parser('metadata')
    meta.add_argument('--tag', default='')
    meta.add_argument('--repo', default=os.environ.get('GITHUB_REPOSITORY', 'MatriQ/su03t-tools'))
    meta.add_argument('--check-release', action='store_true')
    commands.add_parser('macos-archive')
    windows = commands.add_parser('windows-archive')
    windows.add_argument('--directory', type=pathlib.Path, required=True)
    checks = commands.add_parser('checksums')
    checks.add_argument('platform', choices=PLATFORMS)
    pub = commands.add_parser('publish')
    pub.add_argument('--tag', required=True)
    pub.add_argument('--repo', required=True)
    pub.add_argument('--directory', type=pathlib.Path, required=True)
    args = parser.parse_args()
    if args.command == 'metadata': metadata(args)
    elif args.command == 'macos-archive': macos_archive()
    elif args.command == 'windows-archive': windows_archive(args.directory)
    elif args.command == 'checksums': platform_checksums(args.platform)
    elif args.command == 'publish': publish(args)


if __name__ == '__main__':
    main()
