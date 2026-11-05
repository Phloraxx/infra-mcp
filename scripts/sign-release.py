#!/usr/bin/env python3
"""Sign deterministic Infra release manifests. Requires an owner-held signing key.
Never checks secrets into Git; CI reads INFRA_RELEASE_SIGNING_KEY_B64 from GitHub Secrets.
"""
import argparse
import base64
import hmac
import json
import os
from pathlib import Path
from release_contract import collect_assets, validate_tag
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--directory',required=True,type=Path)
    parser.add_argument('--version',required=True)
    parser.add_argument('--source-commit',required=True,help='full tagged Git commit SHA')
    args=parser.parse_args()
    secret=os.environ.get('INFRA_RELEASE_SIGNING_KEY_B64','')
    trusted_public=os.environ.get('INFRA_RELEASE_PUBLIC_KEY_B64','')
    if not secret or not trusted_public:
        parser.error('signing key secret and trusted public key must both be configured')
    private=Ed25519PrivateKey.from_private_bytes(base64.b64decode(secret,validate=True))
    actual_public=base64.b64encode(private.public_key().public_bytes(Encoding.Raw,PublicFormat.Raw)).decode()
    if not hmac.compare_digest(actual_public,trusted_public):
        parser.error('trusted release public key does not match the configured signing key')
    try:
        validate_tag(args.version)
        if len(args.source_commit)!=40 or any(c not in '0123456789abcdefABCDEF' for c in args.source_commit):
            raise ValueError('source commit must be a full 40-character Git SHA')
        assets=collect_assets(args.directory)
    except (ValueError,OSError) as error:
        parser.error(str(error))
    manifest=(json.dumps({'version':args.version,'source_commit':args.source_commit.lower(),'assets':assets},sort_keys=True,separators=(',',':'))+'\n').encode()
    (args.directory/'infra-release.json').write_bytes(manifest)
    signature=base64.b64encode(private.sign(manifest)).decode()+'\n'
    (args.directory/'infra-release.json.sig').write_text(signature)
    print(f'Signed Infra release {args.version} ({len(assets)} binaries).')

if __name__=='__main__':
    main()
