#!/usr/bin/env python3
"""Bootstrap manifest verification acceptance.

Proves the one-command installer verifies the signed release manifest before
it executes any downloaded binary, using a local HTTP server, stub binaries
and a test key. No live install, service or gateway is touched.
"""
import base64
import functools
import http.server
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INSTALL = ROOT / 'scripts/install.sh'
VERIFIER = ROOT / 'scripts/verify-release-manifest.py'
MARKER = "INFRA_MANIFEST_VERIFIER"


def embedded_verifier():
    text = INSTALL.read_text()
    opening = f"<<'{MARKER}'\n"
    start = text.index(opening) + len(opening)
    end = text.index(f'\n{MARKER}\n', start)
    return text[start:end] + '\n'


def load_verifier():
    spec = importlib.util.spec_from_file_location('infra_verify_release_manifest', VERIFIER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def native_target():
    import platform
    system = platform.system().lower().replace('darwin', 'macos')
    machine = platform.machine().lower()
    arch = 'aarch64' if machine in ('arm64', 'aarch64') else 'x86_64'
    return f'{system}-{arch}'


def stub_binaries(directory):
    directory.mkdir(parents=True, exist_ok=True)
    cli = directory / 'infra'
    cli.write_text(
        '#!/bin/sh\n'
        'if [ "$1" = "--build-info" ]; then\n'
        "  printf '%s\\n' '{\"component\":\"infra\",\"activation_protocol\":1,\"version\":\"0.1.1-rc.4\"}'\n"
        '  exit 0\n'
        'fi\n'
        'printf \'%s\\n\' "$@" > "$INFRA_TEST_ARGS"\n')
    cli.chmod(0o755)
    agent = directory / 'infra-agent'
    agent.write_text('#!/bin/sh\nexit 0\n')
    agent.chmod(0o755)


def main():
    assert embedded_verifier() == VERIFIER.read_text(), (
        'the verifier embedded in scripts/install.sh drifted from '
        'scripts/verify-release-manifest.py')

    verifier = load_verifier()
    public = bytes.fromhex('d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a')
    signature = bytes.fromhex(
        'e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155'
        '5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b')
    assert verifier.verify(signature, b'', public), 'RFC 8032 test vector must verify'
    assert not verifier.verify(signature, b'x', public), 'a modified message must not verify'
    assert not verifier.verify(signature[:-1] + b'\x00', b'', public), 'a modified signature must not verify'

    with tempfile.TemporaryDirectory(prefix='infra-install-manifest-') as temporary:
        root = Path(temporary)
        binaries, release, manifests = root / 'binaries', root / 'release', root / 'manifests'
        stub_binaries(binaries)
        key = subprocess.run([sys.executable, str(ROOT / 'scripts/fixture-release.py'),
                              '--binaries', str(binaries), '--directory', str(release),
                              '--manifests', str(manifests)],
                             capture_output=True, text=True, check=True).stdout.strip()
        version = json.loads((release / 'infra-release.json').read_bytes())['version']
        manifest = (release / 'infra-release.json').read_bytes()
        asset = release / f'infra-{native_target()}'
        assert asset.is_file(), f'missing fixture asset {asset.name}'
        asset_bytes = asset.read_bytes()
        arguments = root / 'arguments'

        class Handler(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *_):
                pass

        server = http.server.ThreadingHTTPServer(
            ('127.0.0.1', 0), functools.partial(Handler, directory=str(release)))
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base = f'http://127.0.0.1:{server.server_port}'

        def install(*extra, expect=0, public_key=key, tag=version):
            arguments.unlink(missing_ok=True)
            environment = dict(os.environ, INFRA_BOOTSTRAP_PUBLIC_KEY_B64=public_key,
                               INFRA_TEST_ARGS=str(arguments))
            result = subprocess.run(
                ['sh', str(INSTALL), '--version', tag, '--release-base-url', base,
                 '--install-dir', str(root / 'installed'), '--state-dir', str(root / 'state'),
                 '--no-startup', *extra],
                capture_output=True, text=True, timeout=120, env=environment)
            assert result.returncode == expect, (result.returncode, result.stdout, result.stderr)
            return result

        def restore():
            (release / 'infra-release.json').write_bytes(manifest)
            asset.write_bytes(asset_bytes)

        try:
            accepted = install()
            assert 'Warning: verifying against INFRA_BOOTSTRAP_PUBLIC_KEY_B64' in accepted.stderr
            assert arguments.is_file(), 'the verified installer never ran the staged CLI'
            assert arguments.read_text().splitlines()[0] == 'activate'
            assert '--source-dir' in arguments.read_text().splitlines()

            tampered = bytearray(manifest)
            tampered[len(tampered) // 2] ^= 0x01
            (release / 'infra-release.json').write_bytes(bytes(tampered))
            refused = install(expect=1)
            assert 'did not verify' in refused.stderr, refused.stderr
            assert not arguments.exists(), 'a tampered manifest still executed a binary'
            restore()

            asset.write_bytes(asset_bytes[:-1] + bytes([asset_bytes[-1] ^ 0x01]))
            refused = install(expect=1)
            assert 'did not verify' in refused.stderr, refused.stderr
            assert not arguments.exists(), 'a tampered asset still executed a binary'
            restore()

            refused = install(expect=1, public_key=base64.b64encode(bytes([9] * 32)).decode())
            assert 'did not verify' in refused.stderr, refused.stderr
            assert not arguments.exists(), 'a foreign key still executed a binary'

            refused = install(expect=1, tag='v9.9.9')
            assert 'did not verify' in refused.stderr, refused.stderr
            assert not arguments.exists(), 'a mismatched tag still executed a binary'

            unsafe = install('--allow-checksum-only')
            assert '--allow-checksum-only skips the signed release manifest' in unsafe.stderr
            assert arguments.is_file(), 'the checksum-only fallback did not run the staged CLI'
        finally:
            restore()
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

    print('Bootstrap manifest acceptance passed: the embedded verifier matches '
          'scripts/verify-release-manifest.py and the RFC 8032 vector, a signed '
          'release installs, and a tampered manifest, a tampered asset, a foreign '
          'key or a mismatched tag is refused before any downloaded binary runs.')


if __name__ == '__main__':
    main()
