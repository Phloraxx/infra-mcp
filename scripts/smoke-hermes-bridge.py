#!/usr/bin/env python3
"""Isolated, loopback-only E2E test of the Hermes MCP bridge.

Uses a fake authenticated Hermes HTTP server and disposable Hermes memory home.
No real API secrets, production calls, agent model runs or permanent writes.
Requires: HERMES_TEST_SOURCE and HERMES_TEST_PYTHON environment variables.
"""
import json
import os
import pathlib
import secrets
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from mcp_fixture import FixtureMcp

ROOT = pathlib.Path(__file__).resolve().parents[1]


class FakeHermes(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.headers.get("Authorization") != "Bearer fake-hermes-test-key":
            self.send_response(401)
            self.end_headers()
            return
        if self.path == "/v1/capabilities":
            data = {"platform": "hermes-agent", "features": {"run_status": True}}
        elif self.path.startswith("/api/sessions?"):
            data = {"object": "list", "data": [], "has_more": False}
        else:
            self.send_response(404)
            self.end_headers()
            return
        payload = json.dumps(data).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, fmt, *args):
        return


def main():
    source = pathlib.Path(os.environ["HERMES_TEST_SOURCE"]).resolve(strict=True)
    python = pathlib.Path(os.environ["HERMES_TEST_PYTHON"]).absolute()
    assert python.is_file(), "Hermes venv Python executable missing"
    assert (source / "tools" / "memory_tool.py").is_file()
    subprocess.run(["cargo", "build", "-q", "-p", "infra-gateway", "--locked"], cwd=ROOT, check=True)

    with tempfile.TemporaryDirectory(prefix="infra-hermes-bridge-") as temp:
        d = pathlib.Path(temp)
        key = d / "hermes-test-env"
        key.write_text("# Isolated fixture, not a real secret\nAPI_SERVER_KEY='fake-hermes-test-key'\nOTHER_KEY=unrelated\n")
        key.chmod(0o600)
        fake = ThreadingHTTPServer(("127.0.0.1", 0), FakeHermes)
        thread = threading.Thread(target=fake.serve_forever, daemon=True)
        thread.start()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        token = secrets.token_urlsafe(32)
        env = {k: v for k, v in os.environ.items() if not k.startswith("INFRA_")}
        env.update({
            "INFRA_GATEWAY_BIND": f"127.0.0.1:{port}",
            "INFRA_DB_PATH": str(d / "gateway.db"),
            "INFRA_ADMIN_TOKEN": token,
            "INFRA_HERMES_ENABLED": "1",
            "INFRA_HERMES_BASE_URL": f"http://127.0.0.1:{fake.server_port}/",
            "INFRA_HERMES_API_ENV_FILE": str(key),
            "INFRA_HERMES_HOME": str(d / "hermes-home"),
            "INFRA_HERMES_SOURCE": str(source),
            "INFRA_HERMES_PYTHON": str(python),
            "INFRA_HERMES_MEMORY_WRITES": "1",
            "INFRA_HERMES_RUNS_ENABLED": "0",
        })
        (d / "hermes-home").mkdir(mode=0o700)
        binary = ROOT / "target" / "debug" / "infra-gateway"
        try:
            with open(d / "gateway.log", "wb") as log:
                proc = subprocess.Popen([str(binary)], cwd=ROOT, env=env, stdout=log, stderr=log)
                try:
                    for _ in range(100):
                        if proc.poll() is not None:
                            raise RuntimeError("Staging gateway failed to start")
                        try:
                            with urllib.request.urlopen(base + "/healthz", timeout=1):
                                break
                        except (OSError, urllib.error.URLError):
                            time.sleep(0.05)
                    else:
                        raise RuntimeError("Staging gateway not ready")
                    rpc = FixtureMcp(base, token)
                    def call(name, args):
                        reply = rpc.tool(name, args)
                        if reply.get("isError"):
                            data = reply.get("structuredContent", {})
                            raise RuntimeError(f"{name} failed: {data.get('error_code', 'unknown')}; {data.get('error', '')}")
                        return reply["structuredContent"]

                    capabilities = call("hermes", {"action": "capabilities"})
                    assert capabilities["platform"] == "hermes-agent"
                    forbidden = rpc.tool("hermes", {"action": "start", "input": "Do not invoke",
                                                    "request_id": "fixture-disabled-1"})
                    assert forbidden["isError"], forbidden
                    assert forbidden["structuredContent"].get("error_code") == "unsupported"
                    sessions = call("hermes", {"action": "sessions", "limit": 1})
                    assert "data" in sessions

                    assert call("hermes_memory", {"action": "read", "target": "memory"})["entries"] == []
                    call("hermes_memory", {"action": "add", "target": "memory",
                                           "content": "Disposable Infra bridge test memory"})
                    entries = call("hermes_memory", {"action": "read", "target": "memory"})["entries"]
                    assert entries == ["Disposable Infra bridge test memory"]
                    call("hermes_memory", {"action": "patch", "target": "memory",
                                           "old_text": "test memory", "content": "temporary fact"})
                    entries = call("hermes_memory", {"action": "read", "target": "memory"})["entries"]
                    assert entries == ["Disposable Infra bridge temporary fact"]
                    call("hermes_memory", {"action": "remove", "target": "memory",
                                           "old_text": "Disposable Infra bridge temporary fact"})
                    assert call("hermes_memory", {"action": "read", "target": "memory"})["entries"] == []
                    print("Hermes MCP E2E passed: authenticated API, sessions, temporary memory CRUD.")
                finally:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait(timeout=5)
        finally:
            fake.shutdown()
            fake.server_close()


if __name__ == "__main__":
    main()
