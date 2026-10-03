// Isolated acceptance binary and test credentials only; normal builds are read-only checked.
import {chromium,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {createServer} from 'node:https';
import {createWriteStream,mkdirSync,readFileSync,writeFileSync,unlinkSync,existsSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),id=randomUUID();
const directory=path.join(root,'.local','data-acceptance',id),data=path.join(directory,'data');
mkdirSync(data,{recursive:true});
const exe=path.join(root,'src-tauri','target','release','sayweave.exe'),port=9239;
const regular=process.argv.includes('--regular'),key='isolated-test-key-'+randomUUID(),requests=[],report={id,checks:[],passed:false};
const delay=ms=>new Promise(r=>setTimeout(r,ms));
let app,browser,server;

async function command(command,args,env={}){
 const child=spawn(command,args,{cwd:root,windowsHide:true,env:{...process.env,...env},stdio:['ignore','pipe','pipe']});
 const log=createWriteStream(path.join(directory,'fixture-generation.log'));child.stdout.pipe(log);child.stderr.pipe(log);
 const exit=await new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',resolve);});
 if(exit!==0)throw Error('Fixture generation failed; see local log');
}
async function start(round){
 if(await fetch('http://127.0.0.1:'+port+'/json/version').then(()=>true).catch(()=>false))throw Error('Owned test port occupied; no other process stopped.');
 app=spawn(exe,[],{cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:regular?'':id,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+port+' --remote-debugging-address=127.0.0.1'},stdio:['ignore','pipe','pipe']});
 app.stdout.pipe(createWriteStream(path.join(directory,'round-'+round+'.log')));app.stderr.pipe(createWriteStream(path.join(directory,'round-'+round+'-errors.log')));
 let endpoint;
 for(let i=0;i<200;i++){if(app.exitCode!==null)throw Error('Application exited before ready');endpoint=await fetch('http://127.0.0.1:'+port+'/json/version').then(r=>r.json()).catch(()=>null);if(endpoint)break;await delay(100);}
 if(!endpoint)throw Error('Native WebView unavailable');
 browser=await chromium.connectOverCDP(endpoint.webSocketDebuggerUrl);
 let page;for(let i=0;i<100;i++){page=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('tauri')&&!p.url().includes('surface=overlay'));if(page)break;await delay(100);}
 if(!page)throw Error('Main page missing');await page.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke);
 await page.getByRole('button',{name:'配置模型',exact:true}).click();
 await expect(page.getByRole('heading',{name:'模型',exact:true})).toBeVisible();
 return page;
}
async function stop(page,graceful=true){
 if(app&&app.exitCode===null&&!regular&&page&&graceful)await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('acceptance_exit')).catch(()=>{});
 if(app&&app.exitCode===null&&!regular&&graceful){for(let i=0;i<60&&app.exitCode===null;i++)await delay(100);if(app.exitCode===null)throw Error('Graceful exit exceeded 6 seconds');expect(app.exitCode).toBe(0);}
 if(browser){await browser.close().catch(()=>{});browser=null;}
 if(app&&app.exitCode===null){const owned=app;owned.kill();await new Promise(r=>{if(owned.exitCode!==null)r();else owned.once('exit',r);});}
 app=null;await delay(600);
}
const invoke=(page,command,args={})=>page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
async function fill(page,name,url,model,keyValue=key){
 await page.getByLabel('配置名称',{exact:true}).fill(name);
 await page.getByLabel('服务基址',{exact:true}).fill(url);
 await page.getByLabel('API Key',{exact:true}).fill(keyValue);
 await page.getByLabel('模型 ID',{exact:true}).fill(model);
}
try{
 let base;
 if(!regular){
  await command('cargo',['test','--manifest-path','src-tauri/Cargo.toml','model_desktop_fixture','--','--ignored'],{SAYWEAVE_DATA_ACCEPTANCE_ID:id});
  server=createServer({cert:readFileSync(path.join(data,'fixture-ca.pem')),key:readFileSync(path.join(data,'fixture-key.pem'))},async(req,res)=>{
   let body='';for await(const chunk of req){body+=chunk;if(body.length>8192){res.destroy();return;}}
   const parsed=body?JSON.parse(body):null;
   requests.push({path:req.url,authenticated:req.headers.authorization==='Bearer '+key,model:parsed?.model??null,budget:parsed?.max_tokens??parsed?.max_completion_tokens??null});
   res.setHeader('Content-Type','application/json');
   if(req.url.startsWith('/slow/'))await delay(1200);
   if(req.url==='/compat/chat/completions'&&parsed?.max_tokens){res.writeHead(400);res.end(JSON.stringify({error:{code:'unsupported_parameter',param:'max_tokens'}}));return;}
   if(req.url?.endsWith('/models'))res.end(JSON.stringify({data:[{id:'fixture-text-model'},{id:'fixture-text-model'},{id:'second-model'}]}));
   else res.end(JSON.stringify({choices:[{message:{content:'OK'}}]}));
  });
  await new Promise(r=>server.listen(0,'127.0.0.1',r));
  base='https://localhost:'+server.address().port;
 }
 let page=await start(1);
 await expect(page.getByRole('link',{name:'组件展示'})).toHaveCount(0);
 await expect(page.getByText(/浏览器模拟/)).toHaveCount(0);
 if(regular){
  const status=await invoke(page,'model_status',{retry:false});expect(status.state).toBe('ready');
  const overview=await invoke(page,'model_overview');expect(overview.defaults).toHaveLength(2);
  report.checks.push('ordinary release native models usable; DEV catalogue and acceptance overrides absent');
 }else{
  await expect(page.getByRole('heading',{name:'添加 API 模型'})).toBeVisible();
  await fill(page,'个人文字配置',base+'/v1','fixture-text-model');
  // Listing is usable before a model/name is supplied.
  await page.getByLabel('配置名称',{exact:true}).fill('');await page.getByLabel('模型 ID',{exact:true}).fill('');
  await page.getByRole('button',{name:'获取模型列表'}).click();await expect(page.getByText('列表已获取；选择或手动填写模型 ID。')).toBeVisible();
  await page.getByLabel('配置名称',{exact:true}).fill('个人文字配置');await page.getByLabel('模型 ID',{exact:true}).fill('fixture-text-model');
  await page.getByRole('button',{name:'测试文字模型'}).click();await expect(page.getByText(/本次文字测试通过/)).toBeVisible();
  await page.getByRole('checkbox',{name:'语音识别',exact:true}).check();
  await page.getByRole('button',{name:'保存配置'}).click();await expect(page.getByText('配置已保存。')).toBeVisible();
  let overview=await invoke(page,'model_overview');expect(overview.configs).toHaveLength(1);expect(overview.defaults.every(d=>d.configId===overview.configs[0].id)).toBe(true);
  expect(JSON.stringify(overview)).not.toContain(key);
  report.checks.push('real vault/disk save; independent default slots; HTTPS list and concrete text test');
  const first=overview.configs[0];
  await page.getByRole('button',{name:/个人文字配置.*fixture-text-model/}).click();
  await expect(page.getByLabel('API Key',{exact:true})).toHaveValue('');
  await page.getByLabel('服务基址',{exact:true}).fill('https://other.invalid/v1');
  await page.getByRole('button',{name:'测试文字模型'}).click();await expect(page.getByText(/服务主机或端口已变化/)).toBeVisible();
  await page.getByLabel('服务基址',{exact:true}).fill(base+'/compat');
  await page.getByRole('button',{name:'测试文字模型'}).click();await expect(page.getByText(/本次文字测试通过/)).toBeVisible();
  await page.getByRole('button',{name:'保存配置'}).click();await expect(page.getByText('配置已保存。')).toBeVisible();
  await page.getByRole('button',{name:'添加模型'}).click();await fill(page,'第二份文字配置',base+'/v1','second-model');
  await page.getByRole('button',{name:'保存配置'}).click();await expect(page.getByText('配置已保存。')).toBeVisible();
  overview=await invoke(page,'model_overview');const second=overview.configs.find(c=>c.id!==first.id);
  expect(overview.defaults.every(d=>d.configId===first.id)).toBe(true);
  await page.getByLabel('文字处理默认配置',{exact:true}).selectOption(second.id);
  await expect(page.getByText('默认配置已更新。')).toBeVisible();
  const overlay=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('surface=overlay'));expect(overlay).toBeTruthy();
  const denied=await overlay.evaluate(async()=>{try{await window.__TAURI_INTERNALS__.invoke('model_overview');return null;}catch(e){return e;}});
  expect(denied).toMatchObject({code:'forbidden'});report.checks.push('stored key never revealed; origin guard; one bounded max_tokens compatibility retry; overlay denied');
  const green=await page.locator('body *').evaluateAll(nodes=>nodes.filter(n=>Array.from(n.childNodes).some(c=>c.nodeType===3&&c.textContent?.trim())&&getComputedStyle(n).color==='rgb(117, 251, 145)').length);expect(green).toBe(0);
  report.viewport=await page.evaluate(()=>({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,overflow:document.documentElement.scrollWidth>innerWidth}));expect(report.viewport.overflow).toBe(false);
  await page.screenshot({path:path.join(directory,'models-saved.png')});
  await stop(page);page=await start(2);
  overview=await invoke(page,'model_overview');expect(overview.configs).toHaveLength(2);expect(overview.configs.every(c=>c.credentialStatus==='present')).toBe(true);
  expect(overview.defaults.find(d=>d.usage==='text').configId).toBe(second.id);
  await page.getByRole('button',{name:/个人文字配置.*fixture-text-model/}).click();await page.getByRole('button',{name:'测试文字模型'}).click();await expect(page.getByText(/本次文字测试通过/)).toBeVisible();
  await page.getByLabel('服务基址',{exact:true}).fill(base+'/slow');await page.getByRole('button',{name:'测试文字模型'}).click();await page.getByRole('button',{name:'取消请求',exact:true}).click();await expect(page.getByText('请求已取消。')).toBeVisible();await delay(1300);await expect(page.getByText(/本次文字测试通过/)).toHaveCount(0);
  await page.getByRole('button',{name:'取消编辑'}).click();await page.getByRole('button',{name:'放弃修改'}).click();
  await page.getByRole('button',{name:'删除',exact:true}).first().click();
  await expect(page.getByRole('alertdialog')).toContainText('语音识别默认将清空');
  await page.getByRole('button',{name:'确认删除'}).click();await expect(page.getByText('配置已删除。')).toBeVisible();
  await stop(page);page=await start(3);
  overview=await invoke(page,'model_overview');expect(overview.configs).toHaveLength(1);expect(overview.configs[0].id).toBe(second.id);expect(overview.defaults.find(d=>d.usage==='speech').configId).toBeNull();
  await page.getByRole('button',{name:'删除',exact:true}).click();await expect(page.getByRole('alertdialog')).toContainText('文字处理默认将清空');
  await page.getByRole('button',{name:'确认删除'}).click();await expect(page.getByText('配置已删除。')).toBeVisible();
  report.checks.push('three real process launches with graceful exits, retained credentials/defaults, cancellation and physical deletion');
  await stop(page);page=null;
  expect(requests.every(r=>r.authenticated)).toBe(true);expect(requests.filter(r=>r.path.endsWith('/chat/completions')).every(r=>r.budget===128)).toBe(true);
  const db=readFileSync(path.join(data,'dictionary.sqlite'));expect(db.includes(Buffer.from(key))).toBe(false);
  for(let round=1;round<=3;round++)for(const name of ['round-'+round+'.log','round-'+round+'-errors.log'])expect(readFileSync(path.join(directory,name),'utf8')).not.toContain(key);
  if(existsSync(path.join(data,'logs','sayweave.log')))expect(readFileSync(path.join(data,'logs','sayweave.log'),'utf8')).not.toContain(key);
  report.network=requests;report.checks.push('database, IPC and whitelisted logs contain no fixture API key; test configs removed via normal UI');
 }
 report.passed=true;
 await stop(page);
}catch(e){report.error=String(e).replaceAll(key,"[redacted]");process.exitCode=1;}finally{
 if(browser){try{const main=browser.contexts().flatMap(c=>c.pages()).find(p=>!p.url().includes('surface=overlay'));if(main&&!regular){const view=await invoke(main,'model_overview').catch(()=>null);if(view)for(const c of view.configs){const current=await invoke(main,'model_overview');const replacements=current.defaults.filter(d=>d.configId===c.id).map(d=>({usage:d.usage,configId:current.configs.find(x=>x.id!==c.id&&x.usages.includes(d.usage))?.id??null}));await invoke(main,'model_remove',{request:{id:c.id,revision:c.revision,expectation:{defaults:current.defaults,replacements}}}).catch(()=>{});}}}catch{/* Best-effort UI cleanup; explicit scoped vault cleanup below is authoritative. */}}
 await stop(null,false);
 if(!regular){try{await command('cargo',['test','--manifest-path','src-tauri/Cargo.toml','model_desktop_cleanup','--','--ignored'],{SAYWEAVE_DATA_ACCEPTANCE_ID:id});report.credentialCleanup=true;}catch{report.credentialCleanup=false;report.passed=false;process.exitCode=1;}}
 if(server){server.closeAllConnections();await new Promise(r=>server.close(r));}
 for(const name of ['fixture-ca.der','fixture-ca.pem','fixture-key.pem'])if(existsSync(path.join(data,name)))unlinkSync(path.join(data,name));
 writeFileSync(path.join(directory,'report.json'),JSON.stringify(report,null,2));writeFileSync(path.join(root,'.local',regular?'models-release-report.json':'models-desktop-report.json'),JSON.stringify(report,null,2));
}

console.log(JSON.stringify(report,null,2));
