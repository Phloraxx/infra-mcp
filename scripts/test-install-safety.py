#!/usr/bin/env python3
"""Native pair/rollback acceptance using owned files; live mode is Windows CI only."""
import argparse,ctypes,hashlib,json,os,pathlib,shutil,subprocess,tempfile,http.server,threading,time,functools,platform
from mcp_fixture import FixtureMcp

ROOT=pathlib.Path(__file__).resolve().parents[1]
EXT='.exe' if os.name=='nt' else ''
CLI='infra'+EXT
AGENT='infra-agent'+EXT

def run(command,expected=0,timeout=120):
    result=subprocess.run([str(arg) for arg in command],capture_output=True,text=True,timeout=timeout)
    assert result.returncode==expected, (result.returncode,result.stdout,result.stderr)
    return result

def hashes(directory):
    return {name:hashlib.sha256((directory/name).read_bytes()).hexdigest() for name in (CLI,AGENT)}

def stub_agent(directory,version,delayed=False):
    code=directory/'agent-fixture.rs'
    info=json.dumps({'component':'infra-agent','activation_protocol':1,'version':version})
    delay = r'''let counter=std::env::current_exe().unwrap().parent().unwrap().join("fixture-probe-count");let count=std::fs::read_to_string(&counter).ok().and_then(|v|v.parse::<u32>().ok()).unwrap_or(0)+1;std::fs::write(counter,count.to_string()).unwrap();if count>1{std::thread::sleep(std::time::Duration::from_secs(3));}''' if delayed else ''
    code.write_text('fn main(){if std::env::args().any(|arg|arg=="--build-info"){'+delay+'println!("{}",r#"'+info+'"#);}else{std::process::exit(17);}}')
    run(['rustc',code,'-o',directory/AGENT])

def fixture_process_alive(pid):
    from ctypes import wintypes
    kernel = ctypes.windll.kernel32
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.OpenProcess(0x00100000, False, pid)  # SYNCHRONIZE; never terminate.
    if not handle:
        return False
    try:
        return kernel.WaitForSingleObject(handle, 0) == 0x102  # WAIT_TIMEOUT
    finally:
        kernel.CloseHandle(handle)


def mcp_body(result):
    content = result.get('structuredContent', {})
    assert not result.get('isError') and content.get('status') == 'ok', content.get('error', content.get('status'))
    return content['body']


def live_updates(args, root, candidate, info):
    assert os.name == 'nt' and args.live_state and args.live_install
    assert args.mcp_base_url and args.mcp_token_file
    owned = args.live_fixture_root.resolve()
    profile, target = args.live_state.resolve(), args.live_install.resolve()
    assert owned.name.startswith('infra-win-smoke-')
    assert profile.is_relative_to(owned) and target.is_relative_to(owned)
    assert args.mcp_token_file.resolve().is_relative_to(owned)
    client = FixtureMcp(args.mcp_base_url, args.mcp_token_file.read_text().strip())
    config = json.loads((profile / 'config.json').read_text())
    assert config['gateway_url'] == client.base.replace('http://', 'ws://') + '/agent'
    assert config['device_name'] == 'Windows Install Smoke'
    original, identity = hashes(target), (profile / 'identity.json').read_bytes()
    device = json.loads(identity)['device_id']
    # Each downloaded source is preflighted twice. Delay the second probe so the
    # MCP caller can finish/close before activation stops its controller. The
    # delay exists only in this compiled test binary, not in production code.
    stub_agent(candidate, info['version'], delayed=True)
    release = root / 'release'
    release.mkdir()
    arch = 'aarch64' if platform.machine().lower() in ('arm64', 'aarch64') else 'x86_64'
    for component in ('infra', 'infra-agent'):
        shutil.copy2(candidate / (component + EXT), release / f'{component}-windows-{arch}.exe')
    public = run(['cargo', 'run', '--manifest-path', ROOT / 'Cargo.toml', '--quiet', '--locked', '-p',
                  'infra-cli', '--example', 'sign_fixture_release', '--', release], timeout=120).stdout.strip()
    requests = []
    class Handler(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            requests.append(self.path)
            super().do_GET()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(release)))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    command = [target / CLI, 'update', '--version', 'v' + info['version'], '--base-url',
               f'http://127.0.0.1:{server.server_port}', '--trusted-update-key', public, '--state-dir', profile]
    quote = lambda value: "'" + str(value).replace("'", "''") + "'"
    script = root / 'invoke fixture update.ps1'
    script.write_text("& " + quote(command[0]) + " " + ' '.join(quote(arg) for arg in command[1:]) + "\nexit $LASTEXITCODE\n")
    shell_command = f'powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{script}"'
    try:
        for mode in ('direct', 'mcp_exec', 'mcp_terminal'):
            session = None
            previous_id = (profile / 'update-current').read_text().strip() if (profile / 'update-current').exists() else None
            try:
                if mode == 'direct':
                    queued = run(command, timeout=40)
                    assert 'pending' in queued.stdout, queued.stdout
                elif mode == 'mcp_exec':
                    result = client.tool('exec', {'device': device, 'command': shell_command, 'timeout_ms': 30_000})
                    body = result['structuredContent']
                    if body.get('status') == 'outcome_unknown' or body.get('error_code') == 'outcome_unknown':
                        assert not body.get('retryable', False)
                        print('MCP exec reported uncertainty; observing its one update without replay.', flush=True)
                    else:
                        assert not result.get('isError') and body.get('exit_code') == 0 and 'pending' in body.get('stdout', ''), (mode, body.get('status'), body.get('error'), body.get('stderr', '')[-2000:])
                else:
                    opened = mcp_body(client.tool('terminal', {'device': device, 'action': 'open', 'command': 'cmd.exe /D /Q', 'cwd': str(root)}))
                    session = opened['session']['id']
                    mcp_body(client.tool('terminal', {'device': device, 'action': 'write', 'session_id': session, 'data': shell_command + '\r\n'}))
                    end = time.monotonic() + 35
                    offset, transcript = 0, ''
                    while time.monotonic() < end:
                        output = mcp_body(client.tool('terminal', {'device': device, 'action': 'read', 'session_id': session, 'offset': offset, 'length': 16384}))
                        offset = output['next_offset']
                        transcript += output['output']
                        if 'is pending.' in transcript: break
                        time.sleep(.1)
                    assert 'is pending.' in transcript, ('terminal did not report pending update', transcript[-2000:])
                    mcp_body(client.tool('terminal', {'device': device, 'action': 'close', 'session_id': session}))
                    meta_path = profile / 'sessions' / session / 'session.json'
                    worker = json.loads(meta_path.read_text()).get('worker_pid')
                    end = time.monotonic() + 10
                    while worker and fixture_process_alive(worker) and time.monotonic() < end:
                        time.sleep(.1)
                    assert not worker or not fixture_process_alive(worker), 'terminal caller worker did not exit'
                update_id = (profile / 'update-current').read_text().strip()
                assert update_id != previous_id, f'{mode}: updater did not create a new request'
                initial = json.loads((profile / 'updates' / f'{update_id}.json').read_text())
                assert initial['phase'] == 'pending', (mode, 'caller did not return while activation was pending', initial.get('phase'))
                assert not list(target.glob('.infra.update.*')) and not list(target.glob('.infra-agent.update.*'))
                started, deadline = time.monotonic(), time.monotonic() + 240
                while time.monotonic() < deadline:
                    try: receipt = json.loads((profile / 'updates' / f'{update_id}.json').read_text())
                    except (OSError, ValueError): receipt = {}
                    if receipt and receipt.get('phase') != 'pending': break
                    if receipt and time.monotonic() - started > 10:
                        assert fixture_process_alive(receipt['owner_pid']), f'{mode}: activation relay died without a completion receipt'
                    time.sleep(.2)
                assert receipt.get('phase') == 'failed', (mode, receipt.get('phase'))
                assert receipt.get('context_verified') is True, f'{mode}: relay identity/privilege context was not verified'
                log = (profile / f'update-{update_id}.log').read_bytes().decode('utf-8', errors='replace')
                assert 'connection_unverified' in log and 'previous pair restored' in log, (mode, log)
                assert hashes(target) == original and (profile / 'identity.json').read_bytes() == identity
                verified = json.loads(run([target / CLI, 'status', '--verify', '--json', '--state-dir', profile], timeout=30).stdout)
                assert verified['connection']['verified'], 'restored agent lacked fresh signed evidence'
                recovered = client.tool('exec', {'device': device, 'command': 'echo infra-update-recovered', 'timeout_ms': 5000})
                assert not recovered.get('isError') and 'infra-update-recovered' in recovered['structuredContent'].get('stdout', ''), 'restored MCP execution failed'
                print(f'Windows {mode} update PASS: caller finished, one update rolled back, stable identity and fresh MCP operation.', flush=True)
            finally:
                if session:
                    try: client.tool('terminal', {'device': device, 'action': 'close', 'session_id': session}, timeout=10)
                    except Exception: pass  # Owned CI runner finally also uninstalls fixture workers.
    finally:
        server.shutdown(); server.server_close(); thread.join(timeout=5)


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--binaries',type=pathlib.Path,default=ROOT/'target'/'debug')
    parser.add_argument('--live-fixture-root',type=pathlib.Path)
    parser.add_argument('--live-state',type=pathlib.Path)
    parser.add_argument('--live-install',type=pathlib.Path)
    parser.add_argument('--mcp-base-url')
    parser.add_argument('--mcp-token-file',type=pathlib.Path)
    args=parser.parse_args()
    binaries=args.binaries.resolve()
    info=json.loads(run([binaries/CLI,'--build-info']).stdout)
    with tempfile.TemporaryDirectory(prefix='infra-pair-test-') as temp:
        root=pathlib.Path(temp)
        installed=root/'installed space λ';state=root/'profile space';candidate=root/'candidate'
        for directory in (installed,state,candidate): directory.mkdir()
        for name in (CLI,AGENT): shutil.copy2(binaries/name,installed/name)
        shutil.copy2(binaries/CLI,candidate/CLI)
        stub_agent(candidate,info['version'])
        config={'version':1,'gateway_url':'ws://127.0.0.1:9/agent','device_name':'Fixture label'}
        (state/'config.json').write_text(json.dumps(config))
        identity=b'fixture-metadata-preserved-by-file-transaction'
        (state/'identity.json').write_bytes(identity)
        (installed/'unrelated.txt').write_text('keep')
        before=hashes(installed)
        command=[candidate/CLI,'activate','--source-dir',candidate,'--install-dir',installed,'--state-dir',state,'--no-startup']
        # Candidate files are swapped, then setup rejects changing an enrolled
        # gateway. Both previous binaries/config must be restored.
        failure=run(command+['--gateway','ws://127.0.0.1:10/agent'],expected=1)
        assert 'previous pair restored' in failure.stderr, failure.stderr
        assert hashes(installed)==before
        assert json.loads((state/'config.json').read_text())==config
        assert (state/'identity.json').read_bytes()==identity
        assert not (installed/'.infra-activation.json').exists()
        if os.name=='nt':
            # Share reads so backup/hash work, deny rename/delete so the second
            # replacement really fails after agent replacement succeeds.
            from ctypes import wintypes
            create=ctypes.windll.kernel32.CreateFileW
            create.argtypes=[wintypes.LPCWSTR,wintypes.DWORD,wintypes.DWORD,wintypes.LPVOID,wintypes.DWORD,wintypes.DWORD,wintypes.HANDLE]
            create.restype=wintypes.HANDLE
            handle=create(str(installed/CLI),0x80000000,1,None,3,0,None)
            assert handle!=wintypes.HANDLE(-1).value
            try:
                failure=run(command,expected=1)
                assert 'activate CLI' in failure.stderr, failure.stderr
                assert hashes(installed)==before
            finally:
                ctypes.windll.kernel32.CloseHandle.argtypes=[wintypes.HANDLE]
                ctypes.windll.kernel32.CloseHandle(handle)
        assert (installed/'unrelated.txt').read_text()=='keep'

        if args.live_fixture_root:
            live_updates(args, root, candidate, info)
    print('Native pair acceptance passed: setup-failure rollback, identity/config preservation, unrelated-file retention, and Windows locked-second-swap where available.')

if __name__=='__main__': main()
