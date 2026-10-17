#!/usr/bin/env python3
"""Safe end-to-end Infra cutover acceptance against one enrolled device.

Required environment:
  INFRA_ACCEPT_GATEWAY_HTTP
  INFRA_ACCEPT_DEVICE_ID
  INFRA_ACCEPT_ADMIN_TOKEN

Optional:
  INFRA_ACCEPT_MCP_HOST=localhost
  INFRA_REQUIRE_DESKTOP=0|1
  INFRA_ACCEPT_INTERACTIVE_DESKTOP=0|1
  INFRA_SKIP_DESKTOP=0|1

The script mutates only a unique temporary directory on the target and restores the
clipboard after its desktop round-trip. It does not reboot, sleep, click, stop
services, or modify non-test files.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request
import uuid

GW = os.environ["INFRA_ACCEPT_GATEWAY_HTTP"].rstrip("/")
DEVICE = os.environ["INFRA_ACCEPT_DEVICE_ID"]
TOKEN = os.environ.get("INFRA_ACCEPT_ADMIN_TOKEN") or os.environ["INFRA_ADMIN_TOKEN"]
HOST = os.environ.get("INFRA_ACCEPT_MCP_HOST", "localhost")
REQUIRE_DESKTOP = os.environ.get("INFRA_REQUIRE_DESKTOP", "0") == "1"
INTERACTIVE_DESKTOP = os.environ.get("INFRA_ACCEPT_INTERACTIVE_DESKTOP", "0") == "1"
SKIP_DESKTOP = os.environ.get("INFRA_SKIP_DESKTOP", "0") == "1"

passed: list[str] = []
warnings: list[str] = []
cleanup_errors: list[str] = []
request_id = 0


def note_ok(name: str, detail: str = "") -> None:
    passed.append(name)
    print(f"PASS  {name}{': ' + detail if detail else ''}")


def note_warn(name: str, detail: str) -> None:
    warnings.append(f"{name}: {detail}")
    print(f"WARN  {name}: {detail}")


def http_json(path: str, payload: dict | None = None, timeout: float = 30.0):
    data = None if payload is None else json.dumps(payload).encode()
    headers = {"Authorization": f"Bearer {TOKEN}", "Host": HOST}
    if payload is not None:
        headers.update(
            {
                "Content-Type": "application/json",
                "Accept": "application/json, text/event-stream",
                "MCP-Protocol-Version": "2025-06-18",
            }
        )
    req = urllib.request.Request(GW + path, data=data, headers=headers, method="POST" if payload else "GET")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        body = error.read().decode(errors="replace")
        raise RuntimeError(f"HTTP {error.code} {path}: {body[:500]}") from error


def mcp(tool: str, arguments: dict, timeout: float = 35.0) -> dict:
    global request_id
    request_id += 1
    payload = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "tools/call",
        "params": {"name": tool, "arguments": arguments},
    }
    response = http_json("/mcp", payload, timeout)
    if "error" in response:
        raise RuntimeError(f"MCP {tool}: {response['error']}")
    result = response.get("result") or {}
    structured = result.get("structuredContent") or {}
    if result.get("isError") or structured.get("status") in {"failed", "offline", "deadline_exceeded", "outcome_unknown"}:
        raise RuntimeError(f"MCP {tool} failed: {structured or result}")
    return structured


def exec_ok(command: str, timeout_ms: int = 15_000) -> dict:
    result = mcp("exec", {"device": DEVICE, "command": command, "timeout_ms": timeout_ms})
    if result.get("exit_code") != 0:
        raise RuntimeError(f"exec exit={result.get('exit_code')}: {result.get('stderr', '').strip()}")
    return result


def op(tool: str, **arguments) -> dict:
    return mcp(tool, {"device": DEVICE, **arguments})


def body(result: dict) -> dict:
    value = result.get("body")
    return value if isinstance(value, dict) else {}


def wait_contains(tool: str, arguments: dict, needle: str, attempts: int = 50) -> dict:
    latest: dict = {}
    for _ in range(attempts):
        latest = op(tool, **arguments)
        if needle in json.dumps(latest):
            return latest
        time.sleep(0.1)
    raise RuntimeError(f"{tool} output never contained {needle!r}: {latest}")


session_id = None
job_id = None
upload_id = None
download_id = None
test_root = None
clipboard_original = None

try:
    devices = http_json("/api/devices")
    device = next((row for row in devices if row.get("device_id") == DEVICE), None)
    if not device:
        raise RuntimeError("device is not enrolled in this gateway")
    if not device.get("online"):
        raise RuntimeError("device is enrolled but offline")
    platform = str(device.get("platform") or "").lower()
    capabilities = set(device.get("capabilities") or [])
    note_ok("device-online", f"{device.get('device_name')} generation={device.get('connection_generation')}")

    marker = f"infra-accept-{uuid.uuid4().hex[:12]}"
    exec_result = exec_ok(f"echo {marker}")
    if marker not in exec_result.get("stdout", ""):
        raise RuntimeError("exec stdout marker mismatch")
    note_ok("exec")

    if platform.startswith("win"):
        temp = exec_ok("echo %TEMP%" )["stdout"].strip()
        sep = "\\"
    else:
        temp = exec_ok("printf '%s' \"${TMPDIR:-/tmp}\"")["stdout"].strip() or "/tmp"
        sep = "/"
    test_root = temp.rstrip("/\\") + sep + marker
    text_path = test_root + sep + "sample.txt"
    transfer_path = test_root + sep + "transfer.bin"

    op("files", action="mkdir", path=test_root)
    original_text = f"alpha {marker} alpha\n"
    op("files", action="write", path=text_path, content=original_text, create_parents=True)
    op(
        "files",
        action="exact_edit",
        path=text_path,
        old="alpha",
        new="omega",
        expected_replacements=2,
    )
    read = body(op("files", action="read", path=text_path, offset=0, length=4096))
    expected_text = f"omega {marker} omega\n"
    if read.get("content") != expected_text:
        raise RuntimeError(f"file read/edit mismatch: {read}")
    hashed = body(op("files", action="hash", path=text_path))
    if hashed.get("sha256") != hashlib.sha256(expected_text.encode()).hexdigest():
        raise RuntimeError("file SHA-256 mismatch")
    note_ok("files-read-edit-hash")

    search = op(
        "search",
        root=test_root,
        query=marker,
        mode="content",
        literal=True,
        max_results=20,
        context_lines=1,
    )
    if marker not in json.dumps(search):
        raise RuntimeError("search did not return test marker")
    note_ok("search")

    op("processes", action="list")
    note_ok("process-list")
    op("system", action="summary")
    note_ok("system-summary")
    op("network", action="resolve", host="localhost")
    note_ok("network-resolve")

    terminal_command = "cmd.exe" if platform.startswith("win") else "cat"
    opened = body(op("terminal", action="open", command=terminal_command))
    session_id = opened["session"]["id"]
    terminal_marker = marker + "-pty"
    terminal_input = f"echo {terminal_marker}\r\n" if platform.startswith("win") else terminal_marker + "\n"
    op("terminal", action="write", session_id=session_id, data=terminal_input)
    wait_contains("terminal", {"action": "read", "session_id": session_id, "offset": 0, "length": 32768}, terminal_marker)
    op("terminal", action="resize", session_id=session_id, rows=31, cols=111)
    note_ok("terminal-pty")
    op("terminal", action="close", session_id=session_id)
    session_id = None

    job_start = marker + "-job-start"
    job_end = marker + "-job-end"
    if platform.startswith("win"):
        command = f"echo {job_start} & ping -n 2 127.0.0.1 >nul & echo {job_end}"
    else:
        command = f"printf '{job_start}\\n'; sleep 1; printf '{job_end}\\n'"
    started = body(op("jobs", action="start", command=command, timeout_ms=20_000))
    job_id = started["job"]["id"]
    wait_contains("jobs", {"action": "read", "job_id": job_id, "offset": 0, "length": 32768}, job_end, 80)
    note_ok("durable-job")

    transfer_bytes = (marker.encode() + b"\0INFRA-TRANSFER\xff") * 1024
    transfer_hash = hashlib.sha256(transfer_bytes).hexdigest()
    upload = body(
        op(
            "transfer",
            action="upload_begin",
            path=transfer_path,
            size=len(transfer_bytes),
            sha256=transfer_hash,
            overwrite=True,
            create_parents=True,
        )
    )
    upload_id = upload["transfer"]["id"]
    op(
        "transfer",
        action="upload_chunk",
        transfer_id=upload_id,
        offset=0,
        data_base64=base64.b64encode(transfer_bytes).decode(),
    )
    op("transfer", action="upload_commit", transfer_id=upload_id)
    upload_id = None
    download = body(op("transfer", action="download_begin", path=transfer_path))
    download_id = download["transfer"]["id"]
    chunk = body(
        op(
            "transfer",
            action="download_chunk",
            transfer_id=download_id,
            offset=0,
            length=len(transfer_bytes),
        )
    )
    downloaded = base64.b64decode(chunk["data_base64"])
    if downloaded != transfer_bytes:
        raise RuntimeError("binary transfer round-trip mismatch")
    note_ok("binary-transfer-resume-protocol", f"{len(transfer_bytes)} bytes")
    try:
        op("transfer", action="cancel", transfer_id=download_id)
    except Exception:
        pass
    download_id = None

    if SKIP_DESKTOP:
        note_warn("desktop", "skipped by INFRA_SKIP_DESKTOP=1")
    elif "desktop" in capabilities:
        permissions = body(op("desktop", action="permissions"))
        note_ok("desktop-permission-probe", f"ready={permissions.get('ready')}")
        op("desktop", action="displays")
        op("desktop", action="windows")
        note_ok("desktop-discovery")

        clip = body(op("desktop", action="clipboard_get"))
        clipboard_original = clip.get("text", "")
        clip_marker = marker + "-clipboard"
        op("desktop", action="clipboard_set", text=clip_marker)
        if body(op("desktop", action="clipboard_get")).get("text") != clip_marker:
            raise RuntimeError("clipboard round-trip mismatch")
        op("desktop", action="clipboard_set", text=clipboard_original)
        clipboard_original = None
        note_ok("desktop-clipboard")

        if permissions.get("ready"):
            shot = body(op("desktop", action="screenshot"))
            if not shot.get("data_base64") or shot.get("mime_type") != "image/jpeg":
                raise RuntimeError("desktop screenshot returned no JPEG payload")
            note_ok("desktop-screenshot", f"{shot.get('size')} bytes")
            if INTERACTIVE_DESKTOP:
                op("desktop", action="key", key="escape")
                note_ok("desktop-input-safe-escape")
        else:
            detail = json.dumps(permissions, separators=(",", ":"))
            if REQUIRE_DESKTOP:
                raise RuntimeError(f"desktop permissions are not ready: {detail}")
            note_warn("desktop-permissions", detail)
    elif REQUIRE_DESKTOP:
        raise RuntimeError("desktop capability required but not advertised")
    else:
        note_warn("desktop", "device does not advertise desktop capability")

    print(f"\nCUTOVER_SAFE_ACCEPTANCE_PASS tests={len(passed)} warnings={len(warnings)}")
except Exception as error:
    print(f"\nCUTOVER_SAFE_ACCEPTANCE_FAIL: {error}", file=sys.stderr)
    sys.exit_code = 1
finally:
    if clipboard_original is not None:
        try:
            op("desktop", action="clipboard_set", text=clipboard_original)
        except Exception as error:
            cleanup_errors.append(f"clipboard restore: {error}")
    if session_id:
        try:
            op("terminal", action="close", session_id=session_id)
        except Exception as error:
            cleanup_errors.append(f"terminal close: {error}")
    if job_id:
        try:
            op("jobs", action="cancel", job_id=job_id)
        except Exception:
            pass
    for transfer_id in (upload_id, download_id):
        if transfer_id:
            try:
                op("transfer", action="cancel", transfer_id=transfer_id)
            except Exception:
                pass
    if test_root:
        try:
            op("files", action="delete_permanent", path=test_root, recursive=True)
        except Exception as error:
            cleanup_errors.append(f"temp cleanup: {error}")
    for error in cleanup_errors:
        print(f"WARN  cleanup: {error}", file=sys.stderr)

if getattr(sys, "exit_code", 0):
    raise SystemExit(sys.exit_code)
