import {spawn} from 'node:child_process';
import {writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromium,expect} from '@playwright/test';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const local=path.join(root,'.local');
const profile=process.env.SAYWEAVE_LANGUAGE_PROFILE_ID;
if(!profile)throw Error('SAYWEAVE_LANGUAGE_PROFILE_ID is required');
const port=9255;let browser,child,code;const report={profile,passed:false};
const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
try{
 child=spawn(path.join(root,'src-tauri/target/debug/desktop-acceptance.exe'),[],{cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:profile,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`} });
 child.on('exit',v=>{code=v});
 let endpoint;for(let n=0;n<120;n++){endpoint=await fetch(`http://127.0.0.1:${port}/json/version`).then(r=>r.json()).catch(()=>null);if(endpoint)break;await wait(100);}
 if(!endpoint)throw Error('dashboard WebView unavailable');
 browser=await chromium.connectOverCDP(endpoint.webSocketDebuggerUrl);let page;
 for(let n=0;n<80;n++){page=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('tauri')&&!p.url().includes('surface=overlay'));if(page)break;await wait(100);}
 if(!page)throw Error('dashboard main window missing');await page.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke);
 const native=await page.evaluate(async()=>{const invoke=window.__TAURI_INTERNALS__.invoke;const overview=await invoke('language_overview',{after:null});const details=overview.applications[0]?await invoke('language_profile',{scope:overview.applications[0].scope}):null;return {overview,details};});
 report.native=native;expect(native.overview.preferenceCount).toBeGreaterThan(0);expect(native.overview.applicationCount).toBe(1);expect(native.overview.observationCount).toBe(3);
 await page.getByRole('link',{name:'个性化',exact:true}).click();const dashboard=page.getByRole('region',{name:'个性化数据'});await expect(dashboard.getByText('已学表达习惯',{exact:true}).locator('..').locator('strong')).toHaveText(String(native.overview.preferenceCount));await dashboard.getByRole('button',{name:/Codex/}).click();for(const item of native.details.preferences)await expect(dashboard.getByText(item.rule,{exact:true})).toBeVisible();report.dashboardRendered=true;
 await page.getByRole('link',{name:'词典',exact:true}).click();await expect(page.getByRole('button',{name:'编辑词语 绮蓝',exact:true})).toBeVisible();await page.getByRole('button',{name:'更多操作 绮蓝',exact:true}).click();await page.getByRole('menuitem',{name:'删除',exact:true}).click();await page.getByRole('dialog').getByRole('button',{name:'删除',exact:true}).click();await expect(page.getByText('还没有词条',{exact:true})).toBeVisible();await expect(page.getByText('已删除',{exact:true})).toHaveCount(0);report.dictionaryDelete=true;
 await page.evaluate(()=>window.__TAURI_INTERNALS__.invoke('acceptance_exit'));for(let n=0;n<60&&code===undefined;n++)await wait(100);expect(code).toBe(0);report.passed=true;
}catch(e){report.error=String(e);process.exitCode=1;}finally{if(browser)await browser.close().catch(()=>{});if(child&&code===undefined)child.kill();writeFileSync(path.join(local,'language-dashboard-live.json'),JSON.stringify(report,null,2),'utf8');}
console.log(JSON.stringify(report));
