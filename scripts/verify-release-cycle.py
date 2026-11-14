#!/usr/bin/env python3
"""Signed release cycle acceptance against published artifacts.

Runs the real `infra update` verifier over HTTP against a directory of
published release artifacts and a CI-owned test key. Every scenario uses
`--verify-only`, so nothing is activated, no installed file is replaced and
the recorded release-index floor is never advanced by the check itself.

Scenarios: a fresh device accepts the current release, a device at the
current index accepts the next one, an older signed release below the floor
is refused before any asset is requested, a tampered manifest is refused,
a tampered asset is refused by digest, and a requested-version mismatch is
refused. SemVer downgrade refusal is covered by test-release-policy.py.
"""
import argparse
import functools
import hashlib
import http.server
import json
import os
import platform
import shutil
import subprocess
import tempfile
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXT = '.exe' if os.name == 'nt' else ''
CLI, AGENT = 'infra' + EXT, 'infra-agent' + EXT
LABELS = ('current', 'next', 'old')


def native_target():
    system = platform.system().lower().replace('darwin', 'macos')
    machine = platform.machine().lower()
    arch = 'aarch64' if machine in ('arm64', 'aarch64') else 'x86_64'
    return f'{system}-{arch}'


def run(command, expected=0, timeout=120):
    result = subprocess.run([str(arg) for arg in command], capture_output=True,
                            text=True, timeout=timeout)
    assert result.returncode == expected, (result.returncode, result.stdout, result.stderr)
    return result


def hashes(directory):
    return {name: hashlib.sha256((directory / name).read_bytes()).hexdigest()
            for name in (CLI, AGENT)}


def link_or_copy(source, destination):
    destination.unlink(missing_ok=True)
    try:
        os.link(source, destination)
    except OSError:
        shutil.copy2(source, destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release-dir', required=True, type=Path,
                        help='published artifacts: the twelve binaries and their sidecars')
    parser.add_argument('--public-key-b64', required=True,
                        help='base64 Ed25519 public key that signed the manifests')
    parser.add_argument('--manifests', type=Path,
                        help='directory with current/, next/ and old/ manifest+sig pairs')
    parser.add_argument('--work-dir', type=Path,
                        help='scratch directory; a temporary one is used by default')
    args = parser.parse_args()

    release = args.release_dir.resolve()
    target = native_target()
    work = (args.work_dir or Path(tempfile.mkdtemp(prefix='infra-release-cycle-'))).resolve()
    installed, state, served = work / 'installed', work / 'profile', work / 'served'
    for path in (installed, state, served):
        path.mkdir(parents=True, exist_ok=True)

    for component in ('infra', 'infra-agent'):
        source = release / f'{component}-{target}{EXT}'
        assert source.is_file(), f'missing published asset {source.name} for {target}'
        shutil.copy2(source, installed / (component + EXT))
    (state / 'config.json').write_text(json.dumps({'version': 1, 'gateway_url': 'ws://127.0.0.1:9/agent'}))
    (state / 'identity.json').write_bytes(b'fixture-identity-not-read-by-verification')
    original = hashes(installed)

    manifests = {}
    if args.manifests:
        for label in LABELS:
            directory = (args.manifests / label).resolve()
            for name in ('infra-release.json', 'infra-release.json.sig'):
                assert (directory / name).is_file(), f'missing {label} manifest input {name}'
            manifests[label] = directory
    else:
        manifests['current'] = release

    for entry in release.iterdir():
        if entry.is_file():
            link_or_copy(entry, served / entry.name)

    requests = []

    class Handler(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            requests.append(self.path)
            super().do_GET()

    server = http.server.ThreadingHTTPServer(
        ('127.0.0.1', 0), functools.partial(Handler, directory=str(served)))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base = f'http://127.0.0.1:{server.server_port}'
    floor_path = state / 'release-index.json'
    asset = f'infra-{target}{EXT}'

    def index_of(label):
        return json.loads((manifests[label] / 'infra-release.json').read_bytes())['release_index']

    def verify(label, code=None, tamper=None, requested='latest', floor=None, fetches=None):
        for name in ('infra-release.json', 'infra-release.json.sig'):
            shutil.copy2(manifests[label] / name, served / name)
        if tamper == 'manifest':
            data = bytearray((served / 'infra-release.json').read_bytes())
            data[len(data) // 2] ^= 0x01
            (served / 'infra-release.json').write_bytes(bytes(data))
        elif tamper == 'asset':
            data = bytearray((release / asset).read_bytes())
            data[0] ^= 0x01
            (served / asset).unlink()
            (served / asset).write_bytes(bytes(data))
        floor_path.unlink(missing_ok=True)
        if floor is not None:
            floor_path.write_text(json.dumps(
                {'release_index': floor, 'version': 'fixture', 'recorded_unix_ms': 0}))
        before_floor = floor_path.read_bytes() if floor_path.exists() else None
        requests.clear()
        result = run([installed / CLI, 'update', '--verify-only', '--version', requested,
                      '--base-url', base, '--trusted-update-key', args.public_key_b64,
                      '--state-dir', state], expected=0 if code is None else 1)
        if code is not None:
            assert code in result.stderr, (code, result.stdout, result.stderr)
        if fetches is not None:
            assert len(requests) == fetches, requests
        assert hashes(installed) == original, 'verification changed an installed binary'
        assert not list(installed.glob('.infra-stage-*')), 'staging directory left behind'
        assert not list(installed.glob('.infra-download-*')), 'download directory left behind'
        assert not (state / 'update-current').exists(), 'verification started an activation'
        after_floor = floor_path.read_bytes() if floor_path.exists() else None
        assert after_floor == before_floor, 'verification changed the release index floor'
        if tamper == 'asset':
            link_or_copy(release / asset, served / asset)
        return result

    try:
        accepted = verify('current')
        assert 'verified release' in accepted.stdout, accepted.stdout
        if 'next' in manifests:
            verify('next', floor=index_of('current'))
            verify('old', code='update_rollback_blocked', floor=index_of('next'), fetches=2)
        verify('current', code='manifest signature', tamper='manifest', fetches=2)
        verify('current', code='SHA-256 mismatch', tamper='asset', fetches=3)
        verify('current', code='update_version_mismatch', requested='v999.0.0', fetches=2)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)

    print('Release cycle acceptance passed: signed release verified, next release '
          'accepted above the recorded floor, older signed release refused as '
          'update_rollback_blocked, tampered manifest and tampered asset refused, '
          'requested-version mismatch refused; no installed file, activation or '
          'floor changed.')


if __name__ == '__main__':
    main()
