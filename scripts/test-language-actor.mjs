// Explicit live acceptance: only the saved default text model, synthetic edits,
// isolated data/credentials, production actor and real owned WebView commands.
import {spawn} from 'node:child_process';
import {readFileSync,writeFileSync,existsSync,unlinkSync,mkdirSync} from 'node:fs';
import {randomUUID} from 'node:crypto';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromium,expect} from '@playwright/test';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const local=path.join(root,'.local');
const resultFile=path.join(local,'language-actor-acceptance.json');
const report={passed:false,isolatedProfileId:randomUUID()};
const port=9254;
let browser,child,exitCode,stdout='',stderr='';
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
mkdirSync(local,{recursive:true});
try {
  if(process.platform!=='win32')throw Error('Windows acceptance required');
  if(await fetch(`http://127.0.0.1:${port}/json/version`).then(()=>true).catch(()=>false))throw Error('Owned debug port already occupied');
  if(existsSync(resultFile))unlinkSync(resultFile);
  child=spawn(path.join(root,'src-tauri/target/debug/desktop-acceptance.exe'),[],{
    cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_DESKTOP_ACCEPTANCE:'0',
      SAYWEAVE_DATA_ACCEPTANCE_ID:report.isolatedProfileId,SAYWEAVE_LANGUAGE_ACTOR_ACCEPTANCE:'1',
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`},
  });
  child.stdout.setEncoding('utf8');child.stderr.setEncoding('utf8');
  child.stdout.on('data',value=>{stdout+=value;});child.stderr.on('data',value=>{stderr+=value;});
  child.on('exit',code=>{exitCode=code;});
  child.on('error',error=>{report.launchError=error.message;});
  let endpoint;
  for(let n=0;n<150;n++){
    endpoint=await fetch(`http://127.0.0.1:${port}/json/version`).then(r=>r.json()).catch(()=>null);
    if(endpoint)break;
    if(exitCode!==undefined||report.launchError)throw Error('Owned desktop launch failed');
    await delay(100);
  }
  if(!endpoint)throw Error('Owned WebView debugger unavailable');
  browser=await chromium.connectOverCDP(endpoint.webSocketDebuggerUrl);
  let page;
  for(let n=0;n<100;n++){
    page=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('tauri')&&!p.url().includes('surface=overlay'));
    if(page)break;
    await delay(100);
  }
  if(!page)throw Error('Owned main WebView missing');
  await page.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke);
  let actor;
  const end=Date.now()+210000;
  while(Date.now()<end){
    if(existsSync(resultFile)){
      actor=JSON.parse(readFileSync(resultFile,'utf8'));
      if(actor.isolatedProfileId===report.isolatedProfileId)break;
    }
    if(exitCode!==undefined)throw Error('Owned application exited before actor acceptance');
    await delay(250);
  }
  if(!actor)throw Error('Actor acceptance timed out');
  report.actor=actor;
  if(!actor.passed)throw Error(`Actor acceptance failed: ${actor.error}`);
  report.native=await page.evaluate(async scope=>{
    const invoke=window.__TAURI_INTERNALS__.invoke;
    return {overview:await invoke('language_overview',{after:null}),details:await invoke('language_profile',{scope})};
  },actor.automaticProfile.scope);
  expect(report.native.overview).toEqual(actor.overview);
  expect(report.native.details.version).toBe(actor.automaticProfile.version);
  expect(report.native.details.preferences.length).toBe(actor.automaticProfile.rules);
  await page.getByRole('button',{name:'设置',exact:true}).click();
  const settings=page.getByRole('dialog',{name:'设置',exact:true});
  await settings.getByRole('button',{name:'个性化',exact:true}).click();
  const dashboard=settings.getByRole('region',{name:'个性化数据'});
  for(const [label,value] of [['已学表达习惯',actor.overview.preferenceCount],['已记录应用',1],['学习记录',3]]){
    await expect(dashboard.getByText(label,{exact:true}).locator('..').locator('strong')).toHaveText(String(value));
  }
  await dashboard.getByRole('button',{name:/Codex/}).click();
  for(const preference of report.native.details.preferences)await expect(dashboard.getByText(preference.rule,{exact:true})).toBeVisible();
  for(const minimum of [false,true]){
    await page.evaluate(value=>window.__TAURI_INTERNALS__.invoke('acceptance_main_size',{minimum:value}),minimum);
    await page.screenshot({path:path.join(local,`language-actor-dashboard-${minimum?'960':'1120'}.png`)});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  report.renderedNativeDashboard=true;
  await settings.getByRole('button',{name:'关闭',exact:true}).click();
  await page.getByRole('link',{name:'词典',exact:true}).click();
  await expect(page.getByRole('button',{name:'编辑词语 绮蓝',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'更多操作 绮蓝',exact:true}).click();
  await page.getByRole('menuitem',{name:'删除',exact:true}).click();
  await page.getByRole('dialog').getByRole('button',{name:'删除',exact:true}).click();
  await expect(page.getByText('还没有词条',{exact:true})).toBeVisible();
  await expect(page.getByText('已删除',{exact:true})).toHaveCount(0);
  await page.screenshot({path:path.join(local,'dictionary-delete-native.png')});
  report.nativeDictionaryDelete=true;
  await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('acceptance_exit'));
  for(let n=0;n<70&&exitCode===undefined;n++)await delay(100);
  expect(exitCode).toBe(0);
  report.ownedDesktopExited=true;
  report.passed=true;
}catch(error){report.error=String(error);process.exitCode=1;}
finally{
  if(browser){
    if(exitCode===undefined){
      const page=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('tauri')&&!p.url().includes('surface=overlay'));
      await page?.evaluate(()=>window.__TAURI_INTERNALS__.invoke('acceptance_exit')).catch(()=>{});
      for(let n=0;n<70&&exitCode===undefined;n++)await delay(100);
    }
    await browser.close().catch(()=>{});
  }
  if(child&&exitCode===undefined)child.kill();
  writeFileSync(path.join(local,'language-actor-desktop.log'),stdout,'utf8');
  writeFileSync(path.join(local,'language-actor-desktop-errors.log'),stderr,'utf8');
  writeFileSync(path.join(local,'language-actor-webview.json'),JSON.stringify(report,null,2),'utf8');
}
console.log(JSON.stringify({passed:report.passed,error:report.error,renderedNativeDashboard:report.renderedNativeDashboard,nativeDictionaryDelete:report.nativeDictionaryDelete}));
