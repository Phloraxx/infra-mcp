#!/usr/bin/env python3
"""Build a signed release fixture from locally built binaries.

Assembles the twelve-asset release layout that `scripts/sign-release.py`
requires, using the native pair from --binaries and placeholder bytes for the
other platforms, then signs current/next/old manifests with a public
deterministic test seed. This is a fixture: the seed is in the repository and
must never sign a published release.

Prints the base64 public key on stdout so a caller can feed it to
scripts/verify-release-cycle.py.
"""
import argparse
import base64
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ('linux-x86_64', 'linux-aarch64', 'macos-x86_64',
           'macos-aarch64', 'windows-x86_64', 'windows-aarch64')
TEST_SEED = bytes([31] * 32)


def native_target():
    import platform
    system = platform.system().lower().replace('darwin', 'macos')
    machine = platform.machine().lower()
    arch = 'aarch64' if machine in ('arm64', 'aarch64') else 'x86_64'
    return f'{system}-{arch}'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binaries', required=True, type=Path,
                        help='directory holding the native infra and infra-agent builds')
    parser.add_argument('--directory', required=True, type=Path,
                        help='release directory to create')
    parser.add_argument('--manifests', required=True, type=Path,
                        help='directory to receive current/, next/ and old/ manifests')
    parser.add_argument('--index', type=int, default=41,
                        help='release index for the current manifest')
    args = parser.parse_args()

    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

    key = Ed25519PrivateKey.from_private_bytes(TEST_SEED)
    public = base64.b64encode(
        key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)).decode()
    extension = '.exe' if os.name == 'nt' else ''
    native = native_target()
    release = args.directory.resolve()
    release.mkdir(parents=True, exist_ok=True)
    for component in ('infra', 'infra-agent'):
        source = args.binaries.resolve() / (component + extension)
        assert source.is_file(), f'missing built binary {source}'
        for target in TARGETS:
            name = f'{component}-{target}' + ('.exe' if target.startswith('windows-') else '')
            destination = release / name
            if target == native:
                shutil.copy2(source, destination)
            else:
                destination.write_bytes(f'fixture {name}'.encode())
            digest = hashlib.sha256(destination.read_bytes()).hexdigest()
            (release / (name + '.sha256')).write_text(f'{digest}  {name}')

    version = json.loads(subprocess.run(
        [str(args.binaries.resolve() / ('infra' + extension)), '--build-info'],
        capture_output=True, text=True, check=True).stdout)['version']
    environment = dict(os.environ,
                       INFRA_RELEASE_SIGNING_KEY_B64=base64.b64encode(TEST_SEED).decode(),
                       INFRA_RELEASE_PUBLIC_KEY_B64=public)
    for label, index in (('current', args.index), ('next', args.index + 1),
                         ('old', args.index - 1)):
        assert index >= 1, 'fixture release indexes must stay positive'
        subprocess.run([sys.executable, str(ROOT / 'scripts/sign-release.py'),
                        '--directory', str(release), '--version', 'v' + version,
                        '--source-commit', '1' * 40, '--release-index', str(index)],
                       env=environment, check=True, capture_output=True)
        destination = args.manifests.resolve() / label
        destination.mkdir(parents=True, exist_ok=True)
        for name in ('infra-release.json', 'infra-release.json.sig'):
            shutil.copy2(release / name, destination / name)
    # Leave the current manifest in the release directory, not the last signed one.
    for name in ('infra-release.json', 'infra-release.json.sig'):
        shutil.copy2(args.manifests.resolve() / 'current' / name, release / name)
    print(public)


if __name__ == '__main__':
    main()
