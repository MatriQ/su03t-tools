#!/usr/bin/env python3
"""Release integrity and publication behavior, using local fixtures and fake GH calls."""
import argparse
import contextlib
import importlib.util
import io
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('release_assets', ROOT / 'scripts/release_assets.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseAssetsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='su03t-release-tests-')
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.assets = self.root / 'assets'
        self.assets.mkdir()
        (self.root / 'install.sh').write_text('#!/bin/bash\nexit 0\n')
        (self.root / 'install.ps1').write_text('Write-Output \"test\"\n')
        self.patch = mock.patch.object(release, 'ROOT', self.root)
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.value = '1.2.3'
        for platform in release.PLATFORMS:
            archive = self.assets / release.archive_name(self.value, platform)
            archive.write_bytes(f'package {platform}'.encode())
            release.write_checksums(self.assets / f'SU03T-Tools-v{self.value}-SHA256SUMS-{platform}.txt', [archive])

    def test_complete_assets_and_installer_are_covered_by_one_manifest(self):
        paths = release.assemble(self.assets, self.value)
        manifest = release.parse_checksums(paths[-1].read_text())
        self.assertEqual(set(manifest), {p.name for p in paths[:-1]})
        self.assertEqual(manifest['install.sh'], release.digest(self.root / 'install.sh'))
        self.assertEqual(len(paths), 7)
        self.assertEqual(manifest['install.ps1'], release.digest(self.root / 'install.ps1'))

    def test_partial_build_cannot_be_published(self):
        (self.assets / release.archive_name(self.value, 'Linux-aarch64')).unlink()
        with self.assertRaises(FileNotFoundError):
            release.assemble(self.assets, self.value)

    def test_windows_package_is_required_for_new_releases(self):
        (self.assets / release.archive_name(self.value, 'Windows-x86_64')).unlink()
        with self.assertRaises(FileNotFoundError):
            release.assemble(self.assets, self.value)

    def test_older_release_layout_is_preserved(self):
        value = '0.5.4'
        expected = {'install.sh'}
        for platform in ('macOS-universal', 'Linux-x86_64', 'Linux-aarch64'):
            archive = self.assets / release.archive_name(value, platform)
            archive.write_bytes(platform.encode())
            release.write_checksums(self.assets / f'SU03T-Tools-v{value}-SHA256SUMS-{platform}.txt', [archive])
            expected.add(archive.name)
        paths = release.assemble(self.assets, value)
        self.assertEqual(set(release.parse_checksums(paths[-1].read_text())), expected)

    def test_windows_zip_preserves_nested_unicode_documents(self):
        directory = self.root / 'windows'
        directory.mkdir()
        for name in ('su03t-update.exe', 'su03t-tools.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.txt'):
            (directory / name).write_bytes(b'test')
        document = directory / '.harness/docs/validation.md'
        document.parent.mkdir(parents=True)
        document.write_text('Unicode: 中文', encoding='utf-8')
        with mock.patch.object(release, 'version', return_value=self.value):
            release.windows_archive(directory)
        import zipfile
        archive = self.root / 'dist' / release.archive_name(self.value, 'Windows-x86_64')
        with zipfile.ZipFile(archive) as bundle:
            self.assertEqual(bundle.read('.harness/docs/validation.md').decode('utf-8'), 'Unicode: 中文')

    def test_changed_package_is_rejected_before_combining_checksums(self):
        (self.assets / release.archive_name(self.value, 'macOS-universal')).write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            release.assemble(self.assets, self.value)

    def test_ambiguous_or_unsafe_manifest_is_rejected(self):
        valid = 'a' * 64 + '  package.zip\n'
        for text in (valid + valid, 'a' * 64 + '  ../package.zip\n', '', 'bad hash  package.zip\n'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                release.parse_checksums(text)

    def test_tag_version_mismatch_is_rejected(self):
        with mock.patch.object(release, 'version', return_value=self.value):
            with self.assertRaisesRegex(ValueError, 'does not match'):
                release.metadata(argparse.Namespace(tag='v9.9.9'))

    def test_existing_complete_release_is_not_modified(self):
        with mock.patch.object(release, 'version', return_value=self.value), \
             mock.patch.object(release, 'release_complete', return_value=True), \
             mock.patch.object(release, 'run') as run, contextlib.redirect_stdout(io.StringIO()):
            release.publish(argparse.Namespace(tag='v1.2.3', repo='example/repo', directory=self.assets))
            run.assert_not_called()

    def test_a_published_asset_is_never_silently_replaced(self):
        info = {'draft': False, 'assets': [{'name': release.archive_name(self.value, 'macOS-universal'), 'digest': 'sha256:' + '0' * 64}]}
        with mock.patch.object(release, 'version', return_value=self.value), \
             mock.patch.object(release, 'release_complete', return_value=False), \
             mock.patch.object(release, 'release_info', return_value=info), \
             mock.patch.object(release, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'Refusing to replace'):
                release.publish(argparse.Namespace(tag='v1.2.3', repo='example/repo', directory=self.assets))
            run.assert_not_called()

    def test_an_incomplete_draft_is_not_published(self):
        with mock.patch.object(release, 'version', return_value=self.value), \
             mock.patch.object(release, 'release_complete', return_value=False), \
             mock.patch.object(release, 'release_info', return_value={'draft': True, 'assets': []}), \
             mock.patch.object(release, 'run') as run:
            with self.assertRaisesRegex(RuntimeError, 'leaving the release unpublished'):
                release.publish(argparse.Namespace(tag='v1.2.3', repo='example/repo', directory=self.assets))
            self.assertFalse(any(call.args[0][:3] == ['gh', 'release', 'edit'] for call in run.call_args_list))

    def test_a_new_draft_is_uploaded_and_verified_before_publication(self):
        with mock.patch.object(release, 'version', return_value=self.value), \
             mock.patch.object(release, 'release_complete', side_effect=[False, True, True]), \
             mock.patch.object(release, 'release_info', side_effect=[None, {'draft': True, 'assets': []}]), \
             mock.patch.object(release, 'run') as run, contextlib.redirect_stdout(io.StringIO()):
            release.publish(argparse.Namespace(tag='v1.2.3', repo='example/repo', directory=self.assets))
            commands = [call.args[0] for call in run.call_args_list]
            self.assertEqual([command[2] for command in commands], ['create', 'upload', 'edit'])
            self.assertIn('--draft', commands[0])
            self.assertIn('--clobber', commands[1])
            self.assertIn('--draft=false', commands[2])

    def test_published_release_is_queried_without_listing_drafts(self):
        info = {'tag_name': 'v1.2.3', 'draft': False, 'assets': []}
        with mock.patch.object(subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, json.dumps(info))) as run:
            self.assertEqual(release.release_info('example/repo', 'v1.2.3'), info)
            self.assertEqual(run.call_count, 1)

    def test_draft_is_found_on_a_later_release_list_page(self):
        draft = {'tag_name': 'v1.2.3', 'draft': True, 'assets': []}
        results = [subprocess.CompletedProcess([], 1, '', 'HTTP 404: not found'),
                   subprocess.CompletedProcess([], 0, json.dumps([[{'tag_name': 'v2.0.0'}], [draft]]))]
        with mock.patch.object(subprocess, 'run', side_effect=results) as run:
            self.assertEqual(release.release_info('example/repo', 'v1.2.3'), draft)
            self.assertEqual(run.call_args.args[0],
                             ['gh', 'api', '--paginate', '--slurp', 'repos/example/repo/releases?per_page=100'])

    def test_missing_release_returns_none_after_checking_drafts(self):
        results = [subprocess.CompletedProcess([], 1, '', 'HTTP 404: not found'),
                   subprocess.CompletedProcess([], 0, json.dumps([[{'tag_name': 'v2.0.0'}]]))]
        with mock.patch.object(subprocess, 'run', side_effect=results):
            self.assertIsNone(release.release_info('example/repo', 'v1.2.3'))

    def test_draft_list_errors_are_not_hidden(self):
        results = [subprocess.CompletedProcess([], 1, '', 'HTTP 404: not found'),
                   subprocess.CompletedProcess([], 1, '', 'HTTP 403: forbidden')]
        with mock.patch.object(subprocess, 'run', side_effect=results):
            with self.assertRaisesRegex(RuntimeError, '403'):
                release.release_info('example/repo', 'v1.2.3')

    def test_api_errors_other_than_not_found_are_not_hidden(self):
        result = subprocess.CompletedProcess([], 1, stdout='', stderr='HTTP 403: forbidden')
        with mock.patch.object(subprocess, 'run', return_value=result):
            with self.assertRaisesRegex(RuntimeError, '403'):
                release.release_info('example/repo', 'v1.2.3')


if __name__ == '__main__':
    unittest.main(verbosity=2)
