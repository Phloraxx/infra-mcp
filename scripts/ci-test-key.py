#!/usr/bin/env python3
"""Generate a throwaway Ed25519 key for the CI release cycle.

Writes "<private-seed-b64> <public-key-b64>" to the path given as argv[1].
The key exists only for the duration of one CI run and must never sign a
published release: the owner publisher key is generated offline and stored
only as a GitHub Actions secret. See docs/RELEASE_PUBLISHER_KEY.md.
"""
import base64
import os
import sys

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


def main():
    seed = os.urandom(32)
    key = Ed25519PrivateKey.from_private_bytes(seed)
    public = base64.b64encode(
        key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)).decode()
    with open(sys.argv[1], 'w', encoding='ascii') as handle:
        handle.write(base64.b64encode(seed).decode() + ' ' + public + '\n')


if __name__ == '__main__':
    main()
