import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
const exec=promisify(execFile);
const script=await fs.readFile(new URL('../downloads/install.sh',import.meta.url),'utf8');
const helper=script.slice(script.indexOf('CURL_NETWORK_FLAGS=()'),script.indexOf('# Pairing has a request identifier'));

await test('Unix installer download failure recovery',{skip:process.platform==='win32'},async t=>{
  const work=await fs.mkdtemp(path.join(os.tmpdir(),'infra-download-test-'));
  try{
    await fs.writeFile(path.join(work,'sleep'),'#!/bin/bash\nexit 0\n',{mode:0o700});
    await fs.writeFile(path.join(work,'curl'),`#!/bin/bash
set -eu
n=0; [ ! -f "$TEST_COUNT" ] || n=$(cat "$TEST_COUNT")
n=$((n+1)); printf '%s' "$n" > "$TEST_COUNT"
printf '%s\\n' "$*" >> "$TEST_FLAGS"
out=''
while [ "$#" -gt 0 ]; do
  case "$1" in -o) out="$2"; shift 2 ;; *) shift ;; esac
done
case "$TEST_MODE" in
 tls) if [ "$n" -lt 3 ]; then printf 000; exit 35; fi ;;
 reset) if [ "$n" -eq 1 ]; then printf partial > "$out"; printf 200; exit 56; fi ;;
 exhausted) printf 000; exit 35 ;;
 missing) printf 404; exit 22 ;;
 server) if [ "$n" -eq 1 ]; then printf 503; exit 22; fi ;;
 certificate) printf 000; exit 60 ;;
 disk) printf 200; exit 23 ;;
esac
printf file-ready > "$out"; printf 200
`,{mode:0o700});
    async function run(mode,compat='0'){
      const dir=path.join(work,mode);await fs.mkdir(dir);
      const env={...process.env,PATH:work+path.delimiter+process.env.PATH,INFRA_CURL_COMPAT:compat,TEST_MODE:mode,TEST_COUNT:path.join(dir,'count'),TEST_FLAGS:path.join(dir,'flags'),TEST_DOWNLOAD_OUT:path.join(dir,'result'),TEST_DOWNLOAD_URL:'https://example.invalid/downloads/manifest.json'};
      let result;
      try{result={...(await exec('/bin/bash',['-c',`set -euo pipefail\n${helper}\ndownload "$TEST_DOWNLOAD_URL" "$TEST_DOWNLOAD_OUT"`],{env})),code:0};}
      catch(e){result={stdout:e.stdout,stderr:e.stderr,code:e.code};}
      return {...result,count:Number(await fs.readFile(env.TEST_COUNT,'utf8')),flags:(await fs.readFile(env.TEST_FLAGS,'utf8')).trim().split('\n'),output:await fs.readFile(env.TEST_DOWNLOAD_OUT,'utf8').catch(()=>null)};
    }
    await t.test('TLS reset retries with HTTP 1.1 then IPv4',async()=>{
      const r=await run('tls');assert.equal(r.code,0);assert.equal(r.count,3);assert.equal(r.output,'file-ready');
      assert.doesNotMatch(r.flags[0],/--http1.1|--ipv4/);assert.match(r.flags[1],/--http1.1/);assert.match(r.flags[2],/--ipv4/);
      assert.match(r.stdout,/Downloading manifest.json/);assert.ok(r.flags.every(x=>!x.includes('--insecure')));
    });
    await t.test('partial reset output is overwritten on successful retry',async()=>{const r=await run('reset');assert.equal(r.count,2);assert.equal(r.code,0);assert.equal(r.output,'file-ready');});
    await t.test('HTTP 503 retries',async()=>{const r=await run('server');assert.equal(r.count,2);assert.equal(r.code,0);});
    for(const [mode,message] of [['missing','HTTP 404'],['certificate','curl error 60'],['disk','curl error 23']]){
      await t.test(mode+' fails once with a useful error',async()=>{const r=await run(mode);assert.equal(r.count,1);assert.equal(r.code,1);assert.ok(r.stderr.includes(message));});
    }
    await t.test('persistent resets stop after three attempts before pairing',async()=>{const r=await run('exhausted');assert.equal(r.count,3);assert.equal(r.code,1);assert.match(r.stderr,/manifest.json after 3 attempts/);assert.match(r.stderr,/Pairing has not started/);});
    await t.test('compatibility mode applies IPv4 and HTTP 1.1 from first request',async()=>{const r=await run('compat','1');assert.equal(r.code,0);assert.match(r.flags[0],/--ipv4 --http1.1/);});
  }finally{await fs.rm(work,{recursive:true,force:true});}
});
