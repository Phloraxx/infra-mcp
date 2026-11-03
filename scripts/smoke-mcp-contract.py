#!/usr/bin/env python3
"""Verify exported MCP schemas/results using a disposable loopback gateway."""
import json
import os
import pathlib
import secrets
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from mcp_fixture import FixtureMcp

ROOT = pathlib.Path(__file__).resolve().parents[1]


def main():
    subprocess.run(['cargo', 'build', '-q', '-p', 'infra-gateway', '--locked'], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix='infra-contract-') as directory:
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        token = secrets.token_urlsafe(32)
        env = os.environ.copy()
        # Never inherit a live gateway auth/database configuration into a fixture.
        for key in list(env):
            if key.startswith('INFRA_'):
                del env[key]
        env.update(INFRA_GATEWAY_BIND=f'127.0.0.1:{port}', INFRA_DB_PATH=str(pathlib.Path(directory)/'gateway.db'), INFRA_ADMIN_TOKEN=token)
        binary = ROOT/'target'/'debug'/('infra-gateway.exe' if os.name == 'nt' else 'infra-gateway')
        with open(pathlib.Path(directory)/'gateway.log', 'wb') as log:
            process = subprocess.Popen([str(binary)], cwd=ROOT, env=env, stdout=log, stderr=log)
            try:
                for _ in range(100):
                    if process.poll() is not None:
                        raise RuntimeError('fixture gateway exited')
                    try:
                        with urllib.request.urlopen(base+'/healthz', timeout=1):
                            break
                    except (OSError, urllib.error.URLError):
                        time.sleep(.05)
                else:
                    raise RuntimeError('fixture gateway failed readiness')

                rpc = FixtureMcp(base, token).rpc

                tools = {tool['name']: tool for tool in rpc('tools/list', {})['result']['tools']}
                assert len(tools) == 15 and 'copy' in tools, set(tools)

                def resolved(schema, node):
                    while '$ref' in node:
                        ref = node['$ref']
                        assert ref.startswith('#/'), ref
                        node = schema
                        for part in ref[2:].split('/'):
                            node = node[part.replace('~1', '/').replace('~0', '~')]
                    return node

                for name in ('files','processes','jobs','terminal','transfer','desktop','system','network','services','containers','git'):
                    schema = tools[name]['inputSchema']
                    action = resolved(schema, schema['properties']['action'])
                    assert action['enum'] and all(isinstance(x,str) for x in action['enum']), (name, action)
                    assert schema['properties']['device']['description'], name
                assert 'cmd.exe' in tools['exec']['description']
                assert tools['terminal']['inputSchema']['properties']['session_id']['description']

                def error(name, arguments, code):
                    result = rpc('tools/call', {'name':name,'arguments':arguments})['result']
                    assert result['isError'] is True, result
                    body = result['structuredContent']
                    assert body['error_code'] == code, body
                    assert body['retryable'] is False and body['next_action'], body

                error('system', {'device':'fixture-missing','action':'summary'}, 'device_not_found')
                error('network', {'device':'fixture-missing','action':'tcp_check','host':'localhost'}, 'invalid_argument')
                error('terminal', {'device':'fixture-missing','action':'read','session_id':'invalid'}, 'invalid_argument')
                error('files', {'device':'fixture-missing','action':'list','path':'.','length':0}, 'invalid_argument')
                error('files', {'device':'fixture-missing','action':'list','path':'.','length':1001}, 'invalid_argument')
                error('files', {'device':'fixture-missing','action':'list','path':'.','offset':100001}, 'invalid_argument')
                invalid = rpc('tools/call', {'name':'system','arguments':{'device':'fixture-missing','action':'invented'}})
                assert 'error' in invalid or invalid.get('result',{}).get('isError') is True, invalid
                print('MCP contract smoke passed: 15 tools, action schemas, shell guidance, and actionable HTTP error results.')
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


if __name__ == '__main__':
    main()
