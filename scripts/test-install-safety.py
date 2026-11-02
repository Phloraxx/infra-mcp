#!/usr/bin/env python3
"""Native pair/rollback acceptance using owned files; live mode is Windows CI only."""
import argparse,ctypes,hashlib,json,os,pathlib,shutil,subprocess,tempfile,http.server,threading,time,functools,platform

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

def stub_agent(directory,version):
    code=directory/'agent-fixture.rs'
    info=json.dumps({'component':'infra-agent','activation_protocol':1,'version':version})
    code.write_text('fn main(){if std::env::args().any(|arg|arg=="--build-info"){println!("{}",r#"'+info+'"#);}else{std::process::exit(17);}}')
    run(['rustc',code,'-o',directory/AGENT])

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--binaries',type=pathlib.Path,default=ROOT/'target'/'debug')
    parser.add_argument('--live-fixture-root',type=pathlib.Path)
    parser.add_argument('--live-state',type=pathlib.Path)
    parser.add_argument('--live-install',type=pathlib.Path)
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
            assert os.name=='nt' and args.live_state and args.live_install
            owned=args.live_fixture_root.resolve();profile=args.live_state.resolve();target=args.live_install.resolve()
            assert owned.name.startswith('infra-win-smoke-')
            assert profile.is_relative_to(owned) and target.is_relative_to(owned)
            assert json.loads((profile/'config.json').read_text())['gateway_url'].startswith('ws://127.0.0.1:')
            original=hashes(target);identity=(profile/'identity.json').read_bytes()
            # Exercise signature verification, Windows post-exit handoff, a bad
            # agent's readiness failure, and actual signed recovery of the pair.
            release=root/'release';release.mkdir()
            arch='aarch64' if platform.machine().lower() in ('arm64','aarch64') else 'x86_64'
            for component in ('infra','infra-agent'):
                shutil.copy2(candidate/(component+EXT),release/f'{component}-windows-{arch}.exe')
            public=run(['cargo','run','--manifest-path',ROOT/'Cargo.toml','--quiet','--locked','-p','infra-cli','--example','sign_fixture_release','--',release],timeout=120).stdout.strip()
            class Handler(http.server.SimpleHTTPRequestHandler):
                def log_message(self,*_): pass
            server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(release)))
            thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
            try:
                queued=run([target/CLI,'update','--version','v-fixture','--base-url',f'http://127.0.0.1:{server.server_port}','--trusted-update-key',public,'--state-dir',profile],timeout=40)
                assert 'pending' in queued.stdout,queued.stdout
                update_id=(profile/'update-current').read_text().strip()
                deadline=time.monotonic()+240
                while time.monotonic()<deadline:
                    try: receipt=json.loads((profile/'updates'/f'{update_id}.json').read_text())
                    except (OSError,ValueError): receipt={}
                    if receipt.get('phase')!='pending' and receipt: break
                    time.sleep(.2)
                assert receipt.get('phase')=='failed',receipt
                log=(profile/f'update-{update_id}.log').read_bytes()
                log=log.decode('utf-16' if log.startswith(b'\xff\xfe') else 'utf-8',errors='replace')
                assert 'connection_unverified' in log and 'previous pair restored' in log,log
            finally:
                server.shutdown();server.server_close();thread.join(timeout=5)
            assert hashes(target)==original
            assert (profile/'identity.json').read_bytes()==identity
            verified=json.loads(run([target/CLI,'status','--verify','--json','--state-dir',profile],timeout=30).stdout)
            assert verified['connection']['verified'],verified
            print('Windows failed-readiness rollback restored both binaries, stable identity and fresh signed operation.')
    print('Native pair acceptance passed: setup-failure rollback, identity/config preservation, unrelated-file retention, and Windows locked-second-swap where available.')

if __name__=='__main__': main()
