#!/usr/bin/env python3
"""Sign deterministic Infra release manifests. Requires an owner-held signing key.
Never checks secrets into Git; CI reads INFRA_RELEASE_SIGNING_KEY_B64 from GitHub Secrets.
"""
import argparse
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--directory',required=True,type=Path)
    parser.add_argument('--version',required=True)
    args=parser.parse_args()
    secret=os.environ.get('INFRA_RELEASE_SIGNING_KEY_B64','')
    trusted_public=os.environ.get('INFRA_RELEASE_PUBLIC_KEY_B64','')
    if not secret or not trusted_public:
        parser.error('signing key secret and trusted public key must both be configured')
    private=Ed25519PrivateKey.from_private_bytes(base64.b64decode(secret,validate=True))
    actual_public=base64.b64encode(private.public_key().public_bytes(Encoding.Raw,PublicFormat.Raw)).decode()
    if not hmac.compare_digest(actual_public,trusted_public):
        parser.error('trusted release public key does not match the configured signing key')
    assets={}
    for file in sorted(args.directory.iterdir()):
        if file.is_file() and file.name.startswith(('infra-','infra-agent-')) and not file.name.endswith('.sha256'):
            assets[file.name]=hashlib.sha256(file.read_bytes()).hexdigest()
    if len(assets)!=12:
        parser.error(f'expected exactly 12 binaries for six platforms, found {len(assets)}')
    manifest=(json.dumps({'version':args.version,'assets':assets},sort_keys=True,separators=(',',':'))+'\n').encode()
    (args.directory/'infra-release.json').write_bytes(manifest)
    signature=base64.b64encode(private.sign(manifest)).decode()+'\n'
    (args.directory/'infra-release.json.sig').write_text(signature)
    print(f'Signed Infra release {args.version} ({len(assets)} binaries).')

if __name__=='__main__':
    main()
