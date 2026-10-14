const { chromium } = require('playwright');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '..'), captures = path.join(root, '.test-captures');
fs.mkdirSync(captures, { recursive: true });
let authenticated = false, calls = [], pairingCount = 0, saved = '';
const devices = [
  { device: 'oracle', hostname: 'oracle-vm', os: 'linux', arch: 'arm64', online: true, home: '/home/drvij', version: '0.3.0', startup: 'systemd-user', capabilities: ['shell','files','process','jobs'] },
  { device: 'rose', hostname: 'Rose', os: 'win32', arch: 'x64', online: false, home: 'C:\\Users\\Rose', version: '0.3.0', startup: 'user-login', capabilities: ['shell','files','process','jobs'] }
];
const server = http.createServer(async (req, res) => {
  const json = (data, status = 200) => { res.writeHead(status, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(data)); };
  let raw = ''; for await (const chunk of req) raw += chunk;
  const body = raw ? JSON.parse(raw) : {};
  if (req.url === '/api/session') {
    if (req.method === 'POST') { if (body.password !== 'ui-test') return json({error:'Incorrect password. Try again.'},401); authenticated=true; }
    if (req.method === 'DELETE') authenticated=false;
    return json({authenticated});
  }
  if (req.url.startsWith('/api/') && !authenticated) return json({error:'Sign in to continue.'},401);
  if (req.url === '/api/overview') return json({devices,events:[],server:{version:'0.3.0',memory:50*1048576,endpoint:'https://mcp.phloraxx.us.to/mcp'}});
  if (req.url === '/api/pairing') { pairingCount++; return json({code:'12345ABCDE',expiresAt:Date.now()+600000,base:'https://mcp.phloraxx.us.to'}); }
  if (req.url === '/api/run') {
    calls.push(body);
    if (body.op === 'shell') return json({exitCode:0,stdout:'infra-ok',stderr:''});
    if (body.op === 'files' && body.args.action === 'list') return json({entries:[{name:'test.txt',path:'/home/drvij/test.txt',type:'file',size:5}]});
    if (body.op === 'files' && body.args.action === 'read') return json({content:'hello',truncated:false});
    if (body.op === 'files' && body.args.action === 'write') {saved=body.args.content;return json({ok:true});}
    if (body.op === 'jobs') return json({jobs:[]});
  }
  const target = req.url === '/' ? 'index.html' : req.url === '/ui/app.js' ? 'app.js' : req.url === '/ui/style.css' ? 'style.css' : null;
  if (!target) {res.writeHead(404);return res.end();}
  res.writeHead(200, {'Content-Type':target.endsWith('.html')?'text/html':target.endsWith('.js')?'application/javascript':'text/css'});
  res.end(fs.readFileSync(path.join(root,'public',target)));
});
(async()=>{
  await new Promise(r=>server.listen(18789,'127.0.0.1',r));
  const browser=await chromium.launch({headless:true,executablePath:process.env.INFRA_CHROMIUM||undefined,args:process.env.INFRA_CHROMIUM?['--no-sandbox']:[]});
  const page=await browser.newPage({viewport:{width:1440,height:1000}}), errors=[];
  page.on('pageerror',e=>errors.push(e.message));
  try{
    await page.goto('http://127.0.0.1:18789');await page.locator('#login').waitFor({state:'visible'});
    await page.screenshot({path:path.join(captures,'login.png')});
    await page.locator('#password').fill('wrong');await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.getByText('Incorrect password. Try again.').waitFor();
    await page.locator('#password').fill('ui-test');await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.locator('#dashboard').waitFor({state:'visible'});
    await page.waitForFunction(()=>document.getElementById('device-detail').textContent.includes('oracle-vm'));
    await page.locator('#command').fill('printf infra-ok');await page.getByRole('button',{name:'Run',exact:true}).click();await page.waitForFunction(()=>document.getElementById('console-output').textContent.includes('[exit 0]'));
    assert.equal(calls.filter(c=>c.op==='shell').length,1);
    await page.screenshot({path:path.join(captures,'desktop.png')});
    await page.getByRole('tab',{name:'Files',exact:true}).click();await page.locator('#file-path').fill('/home/drvij');await page.getByRole('button',{name:'Open directory'}).click();await page.getByRole('button',{name:/test.txt/}).click();await page.locator('#file-content').fill('updated');await page.getByRole('button',{name:'Save file'}).click();await page.getByText('File saved.',{exact:true}).waitFor();assert.equal(saved,'updated');
    await page.locator('#add-device').click();await page.locator('#pair-dialog').waitFor({state:'visible'});assert.ok(pairingCount===1);
    await page.locator('#pair-platform').selectOption('macos');
    const macCommand=await page.locator('#install-command').inputValue();
    assert.match(macCommand,/curl -4 --http1\.1 -fsSL/);assert.match(macCommand,/INFRA_CURL_COMPAT=1 bash/);assert.match(macCommand,/--code 12345ABCDE/);assert.match(macCommand,/mktemp/);assert.match(macCommand,/trap 'rm -f/);assert.doesNotMatch(macCommand,/\|\s*bash/);
    await page.evaluate(()=>{Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async value=>{window.copiedInstall=value;}}});});
    await page.locator('#copy-install').click();assert.equal(await page.evaluate(()=>window.copiedInstall),macCommand);
    await page.screenshot({path:path.join(captures,'pairing.png')});
    await page.locator('#pair-platform').selectOption('linux');const linuxCommand=await page.locator('#install-command').inputValue();assert.match(linuxCommand,/curl -fsSL/);assert.match(linuxCommand,/--code 12345ABCDE/);assert.doesNotMatch(linuxCommand,/--ipv4|-4 --http1\.1|INFRA_CURL_COMPAT|\|\s*bash/);
    await page.locator('#pair-platform').selectOption('windows');assert.match(await page.locator('#install-command').inputValue(),/ExecutionPolicy Bypass/);await page.locator('#close-pair').click();
    await page.locator('nav button[data-device="rose"]').click();await page.locator('#offline-banner').waitFor({state:'visible'});assert.equal(await page.locator('#run-command').isDisabled(),true);
    await page.locator('nav button[data-device="oracle"]').click();await page.getByRole('tab',{name:'Console',exact:true}).click();
    await page.setViewportSize({width:390,height:844});await page.screenshot({path:path.join(captures,'mobile.png')});
    const overflow=await page.evaluate(()=>document.documentElement.scrollWidth>window.innerWidth);assert.equal(overflow,false,'Mobile layout overflows');
    await page.locator('#logout').click();await page.locator('#login').waitFor({state:'visible'});assert.deepEqual(errors,[]);
    console.log(JSON.stringify({ok:true,checks:['wrong-password recovery','login','Oracle default selection','command submit exactly once','file read/edit/save','pairing instructions','offline controls','mobile width','logout','no browser exceptions'],captures},null,2));
  }finally{await browser.close();server.close();}
})().catch(e=>{console.error(e);server.close();process.exitCode=1;});
