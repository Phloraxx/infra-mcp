#!/usr/bin/env python3
"""Release packager fixtures with a public deterministic test seed only."""
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from release_contract import expected_assets, package_version

ROOT = Path(__file__).resolve().parents[1]


class ReleaseSigning(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='infra-signing-test-')
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.key = Ed25519PrivateKey.from_private_bytes(bytes([31] * 32))
        self.public = self.key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        self.environment = os.environ.copy()
        self.environment['INFRA_RELEASE_SIGNING_KEY_B64'] = base64.b64encode(bytes([31] * 32)).decode()
        self.environment['INFRA_RELEASE_PUBLIC_KEY_B64'] = base64.b64encode(self.public).decode()
        for name in expected_assets():
            data = ('fixture ' + name).encode()
            (self.directory / name).write_bytes(data)
            (self.directory / (name + '.sha256')).write_text(hashlib.sha256(data).hexdigest() + '  ' + name)

    def sign(self, success=True, version=None, source_commit=None):
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/sign-release.py'),
                                 '--directory', str(self.directory), '--version', version or 'v' + package_version(),
                                 '--source-commit', source_commit or '1' * 40],
                                env=self.environment, capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode == 0, success, result.stderr)
        if not success:
            path = self.directory / 'infra-release.json'
            self.assertTrue(not path.exists() or path.is_symlink())
        return result

    def test_exact_inventory_hashes_signature_and_repeatability(self):
        self.sign()
        manifest = (self.directory / 'infra-release.json').read_bytes()
        signature = base64.b64decode((self.directory / 'infra-release.json.sig').read_text())
        self.key.public_key().verify(signature, manifest)
        data = json.loads(manifest)
        self.assertEqual(set(data['assets']), expected_assets())
        self.assertEqual(data['version'], 'v' + package_version())
        self.assertEqual(data['source_commit'], '1' * 40)
        for name, digest in data['assets'].items():
            self.assertEqual(digest, hashlib.sha256((self.directory / name).read_bytes()).hexdigest())
        self.sign()
        self.assertEqual(manifest, (self.directory / 'infra-release.json').read_bytes())

    def test_twelve_misnamed_files_are_not_a_release(self):
        name = sorted(expected_assets())[0]
        (self.directory / name).rename(self.directory / 'infra-wrong-platform')
        self.assertIn('unexpected release entries', self.sign(False).stderr)

    def test_missing_target_is_rejected(self):
        (self.directory / sorted(expected_assets())[0]).unlink()
        self.assertIn('missing or non-regular', self.sign(False).stderr)

    def test_sidecar_tampering_is_rejected(self):
        name = sorted(expected_assets())[0]
        (self.directory / (name + '.sha256')).write_text('0' * 64 + '  ' + name)
        self.assertIn('checksum sidecar', self.sign(False).stderr)

    def test_empty_binary_is_rejected(self):
        (self.directory / sorted(expected_assets())[0]).write_bytes(b'')
        self.assertIn('empty release binary', self.sign(False).stderr)

    def test_unrelated_publishable_file_is_rejected(self):
        (self.directory / 'unverified.txt').write_text('do not publish')
        self.assertIn('unexpected release entries', self.sign(False).stderr)

    def test_tag_must_match_built_package(self):
        self.assertIn('must match workspace package version', self.sign(False, 'v999.0.0').stderr)

    def test_abbreviated_source_commit_is_rejected(self):
        self.assertIn('full 40-character Git SHA', self.sign(False, source_commit='1234').stderr)

    def test_manifest_symlink_cannot_overwrite_an_unrelated_file(self):
        with tempfile.TemporaryDirectory(prefix='infra-unrelated-test-') as other:
            target = Path(other) / 'keep.txt'
            target.write_text('keep')
            (self.directory / 'infra-release.json').symlink_to(target)
            self.assertIn('non-regular release entry', self.sign(False).stderr)
            self.assertEqual(target.read_text(), 'keep')

    def test_wrong_public_key_is_rejected(self):
        self.environment['INFRA_RELEASE_PUBLIC_KEY_B64'] = base64.b64encode(bytes([9] * 32)).decode()
        self.assertIn('does not match', self.sign(False).stderr)

    def test_absent_secret_is_rejected(self):
        self.environment.pop('INFRA_RELEASE_SIGNING_KEY_B64')
        self.assertIn('must both be configured', self.sign(False).stderr)


if __name__ == '__main__':
    unittest.main()
