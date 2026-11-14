#!/usr/bin/env python3
"""Signed negative-update fixtures; never starts an installed controller."""
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
EXT = '.exe' if os.name == 'nt' else ''
CLI, AGENT = 'infra' + EXT, 'infra-agent' + EXT


def run(args, **kwargs):
    return subprocess.run([str(arg) for arg in args], capture_output=True, text=True, timeout=40, **kwargs)


def main():
    with tempfile.TemporaryDirectory(prefix='infra-release-policy-') as temporary:
        root = Path(temporary)
        installed, state, release = [root / name for name in ('bin space', 'profile', 'release')]
        for path in (installed, state, release): path.mkdir()
        for name in (CLI, AGENT): shutil.copy2(ROOT / 'target/debug' / name, installed / name)
        version = json.loads(run([installed / CLI, '--build-info'], check=True).stdout)['version']
        arch = 'aarch64' if platform.machine().lower() in ('arm64', 'aarch64') else 'x86_64'
        target = f'{platform.system().lower().replace("darwin", "macos")}-{arch}'
        for component in ('infra', 'infra-agent'):
            shutil.copy2(installed / (component + EXT), release / f'{component}-{target}{EXT}')
        config = json.dumps({'version': 1, 'gateway_url': 'ws://127.0.0.1:9/agent'})
        identity = b'fixture-identity-not-read-by-negative-update'
        (state / 'config.json').write_text(config)
        (state / 'identity.json').write_bytes(identity)
        original = {name: hashlib.sha256((installed / name).read_bytes()).hexdigest() for name in (CLI, AGENT)}
        example = run(['cargo', 'build', '--quiet', '--locked', '-p', 'infra-cli', '--example', 'sign_fixture_release'], check=True)
        signer = ROOT / 'target/debug/examples' / ('sign_fixture_release' + EXT)
        requests = []
        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_GET(self):
                requests.append(self.path)
                super().do_GET()
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(release)))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def set_floor(floor):
            (state / 'release-index.json').unlink(missing_ok=True)
            if floor is not None:
                (state / 'release-index.json').write_text(json.dumps(
                    {'release_index': floor, 'version': 'fixture', 'recorded_unix_ms': 0}))

        def check(release_version, requested, code, before_download, tamper_signature=False,
                  release_index=1, floor=None):
            public = run([signer, release, release_version, str(release_index)], check=True).stdout.strip()
            if tamper_signature:
                (release / 'infra-release.json.sig').write_text('invalid-signature')
            set_floor(floor)
            before_floor = (state / 'release-index.json').read_bytes() if (state / 'release-index.json').exists() else None
            requests.clear()
            result = run([installed / CLI, 'update', '--version', requested,
                          '--base-url', f'http://127.0.0.1:{server.server_port}',
                          '--trusted-update-key', public, '--state-dir', state])
            assert result.returncode != 0 and code in result.stderr, (result.returncode, result.stdout, result.stderr)
            if before_download:
                assert requests == ['/infra-release.json', '/infra-release.json.sig'], requests
            else:
                assert len(requests) == 4, requests
            assert {name: hashlib.sha256((installed / name).read_bytes()).hexdigest() for name in (CLI, AGENT)} == original
            assert (state / 'config.json').read_text() == config
            assert (state / 'identity.json').read_bytes() == identity
            assert not (installed / '.infra-activation.json').exists()
            assert not list(installed.glob('.infra-download-*'))
            assert not list(installed.glob('.infra*.update.*'))
            assert not (state / 'update-current').exists()
            after_floor = (state / 'release-index.json').read_bytes() if (state / 'release-index.json').exists() else None
            assert after_floor == before_floor, 'a refused update changed the release index floor'
        def check_verified(release_version, release_index, floor):
            public = run([signer, release, release_version, str(release_index)], check=True).stdout.strip()
            set_floor(floor)
            before_floor = (state / 'release-index.json').read_bytes() if (state / 'release-index.json').exists() else None
            requests.clear()
            result = run([installed / CLI, 'update', '--verify-only', '--version', 'latest',
                          '--base-url', f'http://127.0.0.1:{server.server_port}',
                          '--trusted-update-key', public, '--state-dir', state])
            assert result.returncode == 0, (result.stdout, result.stderr)
            assert 'verified release' in result.stdout, result.stdout
            assert len(requests) == 4, requests
            assert {name: hashlib.sha256((installed / name).read_bytes()).hexdigest() for name in (CLI, AGENT)} == original
            assert not (state / 'update-current').exists()
            after_floor = (state / 'release-index.json').read_bytes() if (state / 'release-index.json').exists() else None
            assert after_floor == before_floor, 'verification changed the release index floor'
            assert not list(installed.glob('.infra-stage-*'))
            assert not list(installed.glob('.infra-download-*'))
        try:
            check('v0.0.0', 'latest', 'update_downgrade_blocked', True)
            check('v-fixture', 'latest', 'invalid_release_version', True)
            check('v' + version, 'v999.0.0', 'update_version_mismatch', True)
            check('v' + version, 'latest', 'manifest signature', True, tamper_signature=True)
            check('v999.0.0', 'latest', 'update_binary_version_mismatch', False)
            # A valid old signature is not enough: the recorded floor refuses it
            # before a single asset is requested.
            check('v' + version, 'latest', 'update_rollback_blocked', True, release_index=4, floor=5)
            check('v' + version, 'latest', 'update_rollback_blocked', True, release_index=1, floor=5)
            # The same release at or above the floor still verifies, and a
            # verification-only run never advances the floor.
            check_verified('v' + version, 5, 5)
            check_verified('v' + version, 6, 5)
            check_verified('v' + version, 1, None)
        finally:
            server.shutdown(); server.server_close(); thread.join(timeout=5)
        print('Release policy acceptance passed: signed replay, malformed versions, requested-version mismatch, bad signatures, signed binary-version mismatch and a signed release below the recorded release index leave pair/config/identity/floor unchanged; a release at or above the floor verifies without activating.')


if __name__ == '__main__':
    main()
