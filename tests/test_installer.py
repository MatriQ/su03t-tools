#!/usr/bin/env python3
"""Exercise installation and failure paths without downloads or real user installs."""
import hashlib
import io
import os
import pathlib
import plistlib
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
REPO = 'https://github.com/MatriQ/su03t-tools'


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='su03t-installer-tests-')
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.fixtures = self.root / 'releases'
        self.prefix = self.root / 'User install with spaces'
        self.stub = self.root / 'commands'
        self.stub.mkdir()
        (self.root / 'tmp').mkdir()
        self.env = os.environ.copy()
        self.env.update(PATH=str(self.stub) + os.pathsep + os.environ['PATH'],
                        TMPDIR=str(self.root / 'tmp'), SU03T_TEST_RELEASES=str(self.fixtures),
                        SU03T_TEST_OS='Linux', SU03T_TEST_ARCH='x86_64')
        (self.stub / 'uname').write_text('#!/bin/bash\ncase "$1" in -s) echo "$SU03T_TEST_OS";; -m) echo "$SU03T_TEST_ARCH";; *) exit 1;; esac\n')
        (self.stub / 'curl').write_text('#!' + sys.executable + '\n' + '''import os,pathlib,shutil,sys
args=sys.argv[1:]
url=next(a for a in args if a.startswith('https://'))
base='https://github.com/MatriQ/su03t-tools/releases/'
if not url.startswith(base): sys.exit(99)
if url==base+'latest':
    print(base+'tag/v1.2.4',end='')
    sys.exit(0)
parts=url.removeprefix(base+'download/').split('/')
if len(parts)!=2:sys.exit(99)
source=pathlib.Path(os.environ['SU03T_TEST_RELEASES'])/parts[0]/parts[1]
if not source.is_file():sys.exit(22)
flag='--output' if '--output' in args else '-o'
shutil.copyfile(source,args[args.index(flag)+1])
''')
        for path in self.stub.iterdir(): path.chmod(0o755)
        for value in ('1.2.3', '1.2.4'):
            for arch in ('x86_64', 'aarch64'): self.linux_package(value, arch)
        self.update_checksums()

    def linux_package(self, value, arch, *, wrong_version=False, unsafe=False, symlink=False):
        directory = self.fixtures / ('v' + value)
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / f'SU03T-Tools-v{value}-Linux-{arch}.tar.gz'
        text = f'#!/bin/sh\nprintf "su03t-update {"0.0.0" if wrong_version else value}\\n"\n'.encode()
        with tarfile.open(path, 'w:gz') as archive:
            binary = tarfile.TarInfo('../escaped.txt' if unsafe else './su03t-update')
            binary.mode = 0o755
            if symlink:
                binary.type = tarfile.SYMTYPE
                binary.linkname = '../outside'
                archive.addfile(binary)
            else:
                binary.size = len(text)
                archive.addfile(binary, io.BytesIO(text))
            license_file = tarfile.TarInfo('./LICENSE')
            license_file.size = len(b'fixture notice')
            archive.addfile(license_file, io.BytesIO(b'fixture notice'))
        return path

    def mac_package(self, value, *, identifier='local.su03t.update', signed=True):
        directory = self.root / ('mac-' + value)
        app = directory / 'SU03T Update.app'
        executable = app / 'Contents/MacOS/su03t-update'
        executable.parent.mkdir(parents=True, exist_ok=True)
        info = {'CFBundleIdentifier': identifier, 'CFBundleExecutable': 'su03t-update',
                'CFBundleName': 'SU03T Update', 'CFBundlePackageType': 'APPL',
                'CFBundleShortVersionString': value, 'CFBundleVersion': '1'}
        (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
        subprocess.run(['cc', '-x', 'c', '-', '-o', str(executable)], input=b'int main(void) { return 0; }\n', check=True, capture_output=True)
        if signed:
            subprocess.run(['codesign', '--force', '--deep', '--sign', '-', str(app)], check=True, capture_output=True)
        archive = self.fixtures / ('v' + value) / f'SU03T-Tools-v{value}-macOS-universal.zip'
        subprocess.run(['ditto', '-c', '-k', '--keepParent', '--norsrc', '--noextattr', str(app), str(archive)], check=True)
        return archive

    def update_checksums(self):
        for directory in self.fixtures.iterdir():
            files = sorted(p for p in directory.iterdir() if p.suffix in ('.gz', '.zip'))
            (directory / f'SU03T-Tools-{directory.name}-SHA256SUMS.txt').write_text(''.join(
                f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n' for path in files))

    def install(self, value=None, *, success=True):
        args = ['/bin/bash', str(ROOT / 'install.sh'), '--prefix', str(self.prefix)]
        if value: args += ['--version', value]
        result = subprocess.run(args, env=self.env, capture_output=True, text=True, timeout=20)
        if success: self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else: self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(list((self.root / 'tmp').iterdir()), 'temporary download files were leaked')
        self.assertFalse((self.prefix / '.su03t-install.lock').exists(), 'installer lock was leaked')
        return result

    def installed_version(self):
        return subprocess.check_output([str(self.prefix / 'bin/su03t-update'), '--version'], text=True).strip()

    def test_linux_latest_works_with_spaces_and_keeps_notices(self):
        self.install()
        self.assertEqual(self.installed_version(), 'su03t-update 1.2.4')
        self.assertEqual((self.prefix / 'share/su03t-tools/v1.2.4/LICENSE').read_text(), 'fixture notice')

    def test_linux_arm64_selects_the_arm_package(self):
        self.env['SU03T_TEST_ARCH'] = 'aarch64'
        result = self.install()
        self.assertIn('Linux-aarch64', result.stdout)

    def test_pinned_version_and_upgrade_keep_the_previous_version(self):
        self.install('v1.2.3')
        self.assertEqual(self.installed_version(), 'su03t-update 1.2.3')
        self.install('v1.2.4')
        self.assertEqual(self.installed_version(), 'su03t-update 1.2.4')
        self.assertTrue((self.prefix / 'share/su03t-tools/v1.2.3/su03t-update').is_file())

    def test_installation_from_the_previous_repository_can_be_reinstalled(self):
        self.install('v1.2.3')
        marker = self.prefix / 'share/su03t-tools/v1.2.3/.su03t-install'
        marker.write_text(marker.read_text().replace('MatriQ/su03t-tools', 'MatriQ/su03t-macos', 1))
        self.install('v1.2.3')
        self.assertEqual(self.installed_version(), 'su03t-update 1.2.3')
        self.assertEqual(marker.read_text().splitlines()[0], 'MatriQ/su03t-tools')

    def test_foreign_installation_marker_is_preserved(self):
        self.install('v1.2.3')
        marker = self.prefix / 'share/su03t-tools/v1.2.3/.su03t-install'
        marker.write_text('example/unrelated-tool\n')
        self.install('v1.2.3', success=False)
        self.assertEqual(self.installed_version(), 'su03t-update 1.2.3')
        self.assertEqual(marker.read_text(), 'example/unrelated-tool\n')

    def test_checksum_failure_preserves_existing_installation(self):
        self.install('v1.2.3')
        archive = self.fixtures / 'v1.2.4/SU03T-Tools-v1.2.4-Linux-x86_64.tar.gz'
        archive.write_bytes(b'corrupted download')
        result = self.install('v1.2.4', success=False)
        self.assertIn('SHA-256 mismatch', result.stderr)
        self.assertEqual(self.installed_version(), 'su03t-update 1.2.3')

    def test_duplicate_manifest_entry_is_rejected(self):
        path = self.fixtures / 'v1.2.4/SU03T-Tools-v1.2.4-SHA256SUMS.txt'
        path.write_text(path.read_text() * 2)
        self.install(success=False)
        self.assertFalse(self.prefix.exists())

    def test_wrong_binary_version_is_rejected_before_install(self):
        self.linux_package('1.2.4', 'x86_64', wrong_version=True)
        self.update_checksums()
        result = self.install(success=False)
        self.assertIn('version does not match', result.stderr)
        self.assertFalse(self.prefix.exists())

    def test_foreign_executable_is_preserved(self):
        target = self.prefix / 'bin/su03t-update'
        target.parent.mkdir(parents=True)
        target.write_text('unrelated local tool')
        self.install(success=False)
        self.assertEqual(target.read_text(), 'unrelated local tool')

    def test_archive_traversal_is_rejected_without_writing_outside_unpack(self):
        self.linux_package('1.2.4', 'x86_64', unsafe=True)
        self.update_checksums()
        self.install(success=False)
        self.assertFalse(list(self.root.rglob('escaped.txt')))
        self.assertFalse(self.prefix.exists())

    def test_archive_symlinks_are_rejected(self):
        self.linux_package('1.2.4', 'x86_64', symlink=True)
        self.update_checksums()
        self.install(success=False)
        self.assertFalse(self.prefix.exists())

    def test_missing_download_does_not_create_an_installation(self):
        (self.fixtures / 'v1.2.4/SU03T-Tools-v1.2.4-Linux-x86_64.tar.gz').unlink()
        self.install(success=False)
        self.assertFalse(self.prefix.exists())

    def test_invalid_version_is_not_used_as_a_url_or_path(self):
        self.install('v1.2.3;echo invalid', success=False)
        self.assertFalse(self.prefix.exists())

    @unittest.skipUnless(sys.platform == 'darwin', 'requires native codesign and ditto')
    def test_macos_installs_and_upgrades_a_verified_bundle(self):
        self.env['SU03T_TEST_OS'] = 'Darwin'
        self.mac_package('1.2.3')
        self.mac_package('1.2.4')
        self.update_checksums()
        self.install('v1.2.3')
        self.install()
        app = self.prefix / 'SU03T Update.app'
        self.assertEqual(plistlib.loads((app / 'Contents/Info.plist').read_bytes())['CFBundleShortVersionString'], '1.2.4')
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)

    @unittest.skipUnless(sys.platform == 'darwin', 'requires native codesign and ditto')
    def test_macos_refuses_a_foreign_bundle(self):
        self.env['SU03T_TEST_OS'] = 'Darwin'
        self.mac_package('1.2.4', identifier='example.other.app')
        self.update_checksums()
        self.install(success=False)
        self.assertFalse(self.prefix.exists())

    @unittest.skipUnless(sys.platform == 'darwin', 'requires native codesign and ditto')
    def test_macos_refuses_an_unsigned_app(self):
        self.env['SU03T_TEST_OS'] = 'Darwin'
        self.mac_package('1.2.4', signed=False)
        self.update_checksums()
        self.install(success=False)
        self.assertFalse(self.prefix.exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
