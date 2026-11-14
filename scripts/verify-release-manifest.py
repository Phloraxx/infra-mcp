#!/usr/bin/env python3
"""Verify a signed Infra release manifest using the standard library only.

The bootstrap installer runs this before executing any downloaded binary, so
it must not need a package manager, a network call or a third-party module.
Ed25519 verification follows RFC 8032 section 6 (the reference implementation
from the RFC appendix, adapted to reject non-canonical inputs).

Exit status 0 means the manifest signature is valid for the pinned public key,
the manifest version matches --expect-version when given, and every requested
asset digest matches the signed manifest. Anything else exits non-zero with a
single actionable line on stderr.
"""
import argparse
import base64
import hashlib
import sys

P = 2**255 - 19
D = -121665 * pow(121666, P - 2, P) % P
Q = 2**252 + 27742317777372353535851937790883648493
I = pow(2, (P - 1) // 4, P)


def inv(value):
    return pow(value, P - 2, P)


def x_recover(y):
    xx = (y * y - 1) * inv(D * y * y + 1)
    x = pow(xx, (P + 3) // 8, P)
    if (x * x - xx) % P != 0:
        x = (x * I) % P
    if x % 2 != 0:
        x = P - x
    return x


def point_add(left, right):
    a = (left[1] - left[0]) * (right[1] - right[0]) % P
    b = (left[1] + left[0]) * (right[1] + right[0]) % P
    c = 2 * left[3] * right[3] * D % P
    d = 2 * left[2] * right[2] % P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def point_mul(scalar, point):
    result = (0, 1, 1, 0)
    while scalar > 0:
        if scalar & 1:
            result = point_add(result, point)
        point = point_add(point, point)
        scalar >>= 1
    return result


def point_equal(left, right):
    if (left[0] * right[2] - right[0] * left[2]) % P != 0:
        return False
    return (left[1] * right[2] - right[1] * left[2]) % P == 0


def on_curve(point):
    x, y, z, t = point
    return (-x * x + y * y - z * z - D * t * t) % P == 0


def point_decompress(data):
    if len(data) != 32:
        return None
    y = int.from_bytes(data, 'little')
    sign = y >> 255
    y &= (1 << 255) - 1
    x = x_recover(y)
    if (x & 1) != sign:
        x = P - x
    point = (x, y, 1, x * y % P)
    return point if on_curve(point) else None


G_Y = 4 * inv(5) % P
G = (x_recover(G_Y), G_Y, 1, x_recover(G_Y) * G_Y % P)


def verify(signature, message, public_key):
    if len(signature) != 64 or len(public_key) != 32:
        return False
    r = point_decompress(signature[:32])
    a = point_decompress(public_key)
    if r is None or a is None:
        return False
    s = int.from_bytes(signature[32:], 'little')
    if s >= Q:
        return False
    digest = hashlib.sha512(signature[:32] + public_key + message).digest()
    h = int.from_bytes(digest, 'little') % Q
    return point_equal(point_mul(s, G), point_add(r, point_mul(h, a)))


def fail(message):
    print(f'infra: {message}', file=sys.stderr)
    raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True)
    parser.add_argument('--signature', required=True)
    parser.add_argument('--public-key-b64', required=True)
    parser.add_argument('--directory', required=True,
                        help='directory the requested assets are read from')
    parser.add_argument('--asset', action='append', default=[],
                        help='asset name to digest-check; repeatable')
    parser.add_argument('--expect-version',
                        help='refuse a manifest whose version differs from this tag')
    args = parser.parse_args()

    try:
        public_key = base64.b64decode(args.public_key_b64.strip(), validate=True)
    except ValueError:
        fail('the pinned release public key is not valid base64')
    if len(public_key) != 32:
        fail('the pinned release public key must be 32 bytes')
    try:
        with open(args.manifest, 'rb') as handle:
            manifest_bytes = handle.read()
        with open(args.signature, 'rb') as handle:
            signature = base64.b64decode(handle.read().strip(), validate=True)
    except (OSError, ValueError) as error:
        fail(f'cannot read the signed release manifest: {error}')
    if not verify(signature, manifest_bytes, public_key):
        fail('the release manifest is not signed by the pinned publisher key; '
             'no downloaded binary was executed')

    import json
    try:
        manifest = json.loads(manifest_bytes)
        version = manifest['version']
        assets = manifest['assets']
    except (ValueError, KeyError, TypeError):
        fail('the signed release manifest is malformed')
    if not isinstance(version, str) or not version:
        fail('the signed release manifest has no version')
    if args.expect_version and args.expect_version != 'latest' and version != args.expect_version:
        fail(f'the signed manifest is for {version}, not the requested {args.expect_version}')
    for name in args.asset:
        expected = assets.get(name) if isinstance(assets, dict) else None
        if not isinstance(expected, str) or len(expected) != 64:
            fail(f'the signed manifest does not list {name}')
        try:
            with open(f'{args.directory}/{name}', 'rb') as handle:
                digest = hashlib.sha256(handle.read()).hexdigest()
        except OSError as error:
            fail(f'cannot read {name}: {error}')
        if digest != expected:
            fail(f'SHA-256 mismatch for {name}: the signed manifest lists {expected}')
    print(version)


if __name__ == '__main__':
    main()
