#!/usr/bin/env python3
"""Offline, isolated native setup/owner-approval/identity-retry acceptance test."""
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

ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "debug"
def get(url, token, *, method="GET", body=None):
    headers={"Authorization": f"Bearer {token}", "X-Infra-CSRF": "1"}
    if body is not None: headers["Content-Type"]="application/json"
    req=urllib.request.Request(url,headers=headers,method=method,data=json.dumps(body).encode() if body is not None else None)
    with urllib.request.urlopen(req,timeout=2) as resp:
        return json.load(resp)

def wait_until(fn, seconds=20):
    deadline=time.monotonic()+seconds
    last=None
    while time.monotonic()<deadline:
        try:
            last=fn()
            if last:return last
        except (urllib.error.URLError, TimeoutError, ValueError):
            pass
        time.sleep(.15)
    raise RuntimeError(f"Condition not met ({last!r})")

def main():
    for p in ("infra","infra-agent","infra-gateway"):
        if not (BIN/p).is_file():raise RuntimeError(f"First build target/debug/{p}")
    with tempfile.TemporaryDirectory(prefix="infra-owner-accept-") as td:
        root=pathlib.Path(td)
        with socket.socket() as s:
            s.bind(("127.0.0.1",0));port=s.getsockname()[1]
        base=f"http://127.0.0.1:{port}"
        token=secrets.token_urlsafe(30)
        env=os.environ.copy()
        env.update(INFRA_GATEWAY_BIND=f"127.0.0.1:{port}",INFRA_DB_PATH=str(root/"gateway.db"),INFRA_ADMIN_TOKEN=token,INFRA_NO_BROWSER="1")
        processes=[]
        def launch(args, **kw):
            proc=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,**kw)
            processes.append(proc)
            return proc
        try:
            server=launch([str(BIN/"infra-gateway")],env=env)
            wait_until(lambda: get(base+"/healthz",token).get("status")=="ok")
            install=root/"bin";state=root/"state"
            installer=launch([str(ROOT/"scripts/install.sh"),"--gateway",f"ws://127.0.0.1:{port}/agent",
                "--source-dir",str(BIN),"--install-dir",str(install),"--state-dir",str(state),"--no-startup"],env=env)
            pending=wait_until(lambda: next(iter(get(base+"/api/pair/requests",token)),None),25)
            assert pending["device_name"] and pending["verification_code"]
            approved=get(base+f"/api/pair/{pending['request_id']}/approve",token,method="POST",body={"verification_code":pending["verification_code"]})
            assert approved["state"]=="approved",approved
            out,err=installer.communicate(timeout=25)
            assert installer.returncode==0,(out.decode(errors="replace"),err.decode(errors="replace"))
            original=(state/"identity.json").read_bytes()
            # No source-dir: repair using installed binaries, without downloading or reenrolling.
            again=subprocess.run([str(ROOT/"scripts/install.sh"),"--install-dir",str(install),"--state-dir",str(state),"--no-startup"],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=25)
            assert again.returncode==0,again.stderr.decode(errors="replace")
            assert original==(state/"identity.json").read_bytes(), "Identity rotated on repair"
            agent=launch([str(install/"infra-agent"),"--state-dir",str(state)],env=env)
            found=wait_until(lambda: next((v for v in get(base+"/api/devices",token) if v.get("device_id")==approved["device_id"] and v.get("online")),None),20)
            assert found["connection_generation"],found
            assert found["device_name"]==pending["device_name"],(found,pending)
            print("Owner pairing acceptance passed: OS hostname, owner approval, persistent signed ID, no-download retry, live agent.")
        finally:
            for proc in reversed(processes):
                if proc.poll() is None:
                    proc.terminate()
            for proc in reversed(processes):
                try:proc.communicate(timeout=3)
                except subprocess.TimeoutExpired:proc.kill();proc.communicate()

if __name__=="__main__":
    main()
