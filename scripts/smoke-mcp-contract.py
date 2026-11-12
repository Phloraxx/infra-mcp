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
                assert len(tools) == 17 and {'copy', 'hermes', 'hermes_memory'} <= set(tools), set(tools)

                devices = rpc('tools/call', {'name':'devices','arguments':{}})['result']
                assert devices['isError'] is False, devices
                assert isinstance(devices['structuredContent'], dict), devices
                assert isinstance(devices['structuredContent'].get('devices'), list), devices

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
                    assert schema['properties']['device']['description'], name
                    if name in ('files','jobs','terminal'):
                        # Each action advertises its own required fields (oneOf).
                        variants = {v['properties']['action']['const']: v for v in schema['oneOf']}
                        assert variants, (name, schema)
                        if name == 'files':
                            assert {'action','path','content'} <= set(variants['write']['required'])
                            assert {'action','path','old','new'} <= set(variants['exact_edit']['required'])
                        if name == 'jobs':
                            assert {'action','command'} <= set(variants['start']['required'])
                            assert {'action','job_id'} <= set(variants['wait']['required'])
                            assert 'wait_timeout_ms' in variants['wait']['properties']
                        if name == 'terminal':
                            assert {'action','session_id','data'} <= set(variants['write']['required'])
                            assert {'action','session_id','rows','cols'} <= set(variants['resize']['required'])
                    else:
                        action = resolved(schema, schema['properties']['action'])
                        assert action['enum'] and all(isinstance(x,str) for x in action['enum']), (name, action)
                assert 'cmd.exe' in tools['exec']['description']
                assert 'compact' in tools['devices']['inputSchema']['properties']
                assert devices['structuredContent']['devices'] == []
                compact = rpc('tools/call', {'name':'devices','arguments':{'compact':True}})['result']
                assert compact['isError'] is False
                assert compact['structuredContent']['devices'] == []

                # Missing variant-specific fields must fail deserialization,
                # not dispatch an incomplete operation to a device.
                for name, arguments in [
                    ('files', {'device':'fixture-missing','action':'write','path':'file'}),
                    ('jobs', {'device':'fixture-missing','action':'wait'}),
                    ('terminal', {'device':'fixture-missing','action':'write','session_id':'unused'}),
                ]:
                    result = rpc('tools/call', {'name':name,'arguments':arguments})
                    assert 'error' in result or result.get('result',{}).get('isError') is True, (name,result)

                def error(name, arguments, code):
                    result = rpc('tools/call', {'name':name,'arguments':arguments})['result']
                    assert result['isError'] is True, result
                    body = result['structuredContent']
                    assert body['error_code'] == code, body
                    assert body['retryable'] is False and body['next_action'], body

                error('hermes', {'action': 'capabilities'}, 'unsupported')
                error('hermes_memory', {'action': 'read', 'target': 'memory'}, 'unsupported')
                error('system', {'device':'fixture-missing','action':'summary'}, 'device_not_found')
                error('network', {'device':'fixture-missing','action':'tcp_check','host':'localhost'}, 'invalid_argument')
                error('terminal', {'device':'fixture-missing','action':'read','session_id':'invalid'}, 'invalid_argument')
                error('files', {'device':'fixture-missing','action':'list','path':'.','length':0}, 'invalid_argument')
                error('files', {'device':'fixture-missing','action':'list','path':'.','length':1001}, 'invalid_argument')
                error('files', {'device':'fixture-missing','action':'list','path':'.','offset':100001}, 'invalid_argument')
                invalid = rpc('tools/call', {'name':'system','arguments':{'device':'fixture-missing','action':'invented'}})
                assert 'error' in invalid or invalid.get('result',{}).get('isError') is True, invalid
                print('MCP contract smoke passed: 17 tools, oneOf action requirements, compact device views and actionable errors.')
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


if __name__ == '__main__':
    main()
