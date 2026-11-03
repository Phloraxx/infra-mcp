"""Small loopback-only MCP client for owned acceptance fixtures, not a product SDK."""
import json
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        return None


class FixtureMcp:
    def __init__(self, base, token):
        address = urllib.parse.urlsplit(base)
        if (address.scheme != 'http' or address.hostname not in ('127.0.0.1', '::1')
                or not address.port or address.username or address.password
                or address.path not in ('', '/') or address.query or address.fragment):
            raise ValueError('MCP fixture endpoint must be an explicit loopback HTTP origin')
        self.base = base.rstrip('/')
        self.token = token
        self.sequence = 0
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def rpc(self, method, params, timeout=40):
        self.sequence += 1
        request = urllib.request.Request(self.base + '/mcp',
            data=json.dumps({'jsonrpc': '2.0', 'id': self.sequence, 'method': method, 'params': params}).encode(),
            headers={'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json',
                     'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-06-18'})
        with self.opener.open(request, timeout=timeout) as response:
            return json.load(response)

    def tool(self, name, arguments, timeout=40):
        response = self.rpc('tools/call', {'name': name, 'arguments': arguments}, timeout)
        if 'error' in response:
            raise AssertionError('fixture MCP JSON-RPC error: ' + str(response['error'].get('code')))
        return response['result']
