#!/usr/bin/env python3
"""Shared release tag, native metadata and six-target artifact contract."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
from functools import lru_cache

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ('linux-x86_64', 'linux-aarch64', 'macos-x86_64',
           'macos-aarch64', 'windows-x86_64', 'windows-aarch64')


@lru_cache(maxsize=1)
def package_version():
    result = subprocess.run(['cargo', 'metadata', '--manifest-path', str(ROOT / 'Cargo.toml'),
                             '--locked', '--no-deps', '--format-version', '1'],
                            capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise ValueError('cannot load Cargo package metadata; ensure the repository toolchain and lockfile are available')
    packages = json.loads(result.stdout)['packages']
    return next(package['version'] for package in packages if package['name'] == 'infra-cli')


def validate_tag(tag):
    expected = 'v' + package_version()
    if tag != expected:
        raise ValueError(f'release tag {tag!r} must match workspace package version {expected}; update Cargo.toml deliberately before tagging')


def expected_assets():
    return {f'{component}-{target}' + ('.exe' if target.startswith('windows-') else '')
            for component in ('infra', 'infra-agent') for target in TARGETS}


def collect_assets(directory):
    expected = expected_assets()
    allowed = expected | {name + '.sha256' for name in expected} | {'infra-release.json', 'infra-release.json.sig'}
    unexpected = {path.name for path in directory.iterdir()} - allowed
    if unexpected:
        raise ValueError(f'unexpected release entries: {sorted(unexpected)}')
    for path in directory.iterdir():
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'non-regular release entry: {path.name}')
    result = {}
    for name in sorted(expected):
        binary = directory / name
        sidecar = directory / (name + '.sha256')
        for path in (binary, sidecar):
            if path.is_symlink() or not path.is_file():
                raise ValueError(f'missing or non-regular release input: {path.name}')
        if binary.stat().st_size == 0:
            raise ValueError(f'empty release binary: {name}')
        hasher = hashlib.sha256()
        with binary.open('rb') as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b''):
                hasher.update(chunk)
        digest = hasher.hexdigest()
        parts = sidecar.read_text(encoding='ascii').split()
        if len(parts) != 2 or parts[0].lower() != digest or parts[1].removeprefix('*') != name:
            raise ValueError(f'checksum sidecar does not match binary/name: {name}')
        result[name] = digest
    return result


def verify_native_pair(directory):
    version = package_version()
    for component in ('infra', 'infra-agent'):
        binary = directory.resolve() / (component + ('.exe' if os.name == 'nt' else ''))
        result = subprocess.run([str(binary), '--build-info'], stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=10, check=True)
        info = json.loads(result.stdout)
        if info.get('component') != component or info.get('version') != version or type(info.get('activation_protocol')) is not int or info['activation_protocol'] != 1:
            raise ValueError(f'{component} build metadata does not match workspace version/activation protocol')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag')
    parser.add_argument('--binaries', type=Path)
    args = parser.parse_args()
    if args.tag is None and args.binaries is None:
        parser.error('provide --tag and/or --binaries; no release was checked')
    try:
        if args.tag is not None:
            validate_tag(args.tag)
        if args.binaries:
            verify_native_pair(args.binaries)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        parser.error(str(error))
    print('Release version/build contract passed.')


if __name__ == '__main__':
    main()
