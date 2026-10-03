// Opt-in acceptance binary only. Connects to its WebView2 test port, not a user's browser.
import {chromium,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {createWriteStream,existsSync,mkdirSync,writeFileSync,readFileSync,copyFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const id=randomUUID();const directory=path.join(root,'.local','data-acceptance',id);mkdirSync(directory,{recursive:true});
const exe=path.join(root,'src-tauri','target','release','sayweave.exe');const report={id,checks:[],ipc:[],viewport:null};
const port=9237;
const regular=process.argv.includes('--regular');
const delay=ms=>new Promise(r=>setTimeout(r,ms));
let child,browser;
async function start(round){
  if(await fetch(`http://127.0.0.1:${port}/json/version`).then(()=>true).catch(()=>false))throw Error('Acceptance debugging port occupied; no process was stopped.');
  child=spawn(exe,[],{cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:regular?'':id,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`},stdio:['ignore','pipe','pipe']});
  child.stdout.pipe(createWriteStream(path.join(directory,`round-${round}.log`)));child.stderr.pipe(createWriteStream(path.join(directory,`round-${round}-errors.log`)));
  let endpoint;
  for(let i=0;i<200;i++){if(child.exitCode!==null)throw Error(`Application exited: ${child.exitCode}`);endpoint=await fetch(`http://127.0.0.1:${port}/json/version`).then(r=>r.json()).catch(()=>null);if(endpoint)break;await delay(150);}
  if(!endpoint)throw Error('WebView2 acceptance connection unavailable');
  browser=await chromium.connectOverCDP(endpoint.webSocketDebuggerUrl);
  let main;
  for(let i=0;i<100;i++){main=browser.contexts().flatMap(c=>c.pages()).find(p=>!p.url().includes('surface=overlay')&&p.url().includes('tauri'));if(main)break;await delay(100);}
  if(!main)throw Error('Main renderer unavailable');
  await main.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke);
  await main.evaluate(()=>{location.hash='/dictionary';});
  await expect(main.getByRole('heading',{name:'词典',exact:true})).toBeVisible();
  return main;
}
async function stop(){if(browser){await browser.close().catch(()=>{});browser=null;}if(child&&child.exitCode===null){child.kill();await new Promise(resolve=>{if(child.exitCode!==null)resolve();else child.once('exit',resolve);});}child=null;await delay(900);}
const invoke=(page,command,args)=>page.evaluate(async({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args});
try{
  let main=await start(1);
  if(regular){
    await expect(main.getByRole('button',{name:'新增词条'})).toBeEnabled();await expect(main.getByRole('link',{name:'组件展示'})).toHaveCount(0);
    await expect(main.getByText('浏览器预览：仅保存在内存，刷新后重置。')).toHaveCount(0);
    report.checks.push('ordinary no-bundle build starts with Vite stopped, dictionary IPC ready, DEV catalogue absent');
    report.viewport=await main.evaluate(()=>({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,overflow:document.documentElement.scrollWidth>innerWidth}));
    expect(report.viewport.overflow).toBe(false);await main.screenshot({path:path.join(root,'tests','visual','dictionary-release-empty.png')});report.passed=true;
  }else{
  await expect(main.getByText('还没有词条')).toBeVisible();
  await expect(main.getByText('浏览器预览：仅保存在内存，刷新后重置。')).toHaveCount(0);
  await expect(main.getByRole('link',{name:'组件展示'})).toHaveCount(0);
  await main.getByRole('button',{name:'新增词条'}).click();let dialog=main.getByRole('dialog');await dialog.getByLabel('词条',{exact:true}).fill('Figma');await dialog.getByLabel('分类（可选）').fill('设计工具');await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(main.getByRole('cell',{name:'Figma',exact:true})).toBeVisible();
  await main.getByRole('button',{name:'新增词条'}).click();await dialog.getByLabel('词条',{exact:true}).fill('figma');await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByRole('alert')).toBeVisible();await expect(dialog.getByLabel('词条',{exact:true})).toHaveValue('figma');await dialog.getByRole('button',{name:'查看已有词条'}).click();await dialog.getByLabel('词条',{exact:true}).fill('Figma Desktop');await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(main.getByRole('cell',{name:'Figma Desktop',exact:true})).toBeVisible();report.checks.push('native term CRUD and duplicate review');
  await main.getByRole('tab',{name:'常见纠错'}).click();await main.getByRole('button',{name:'新增纠错'}).click();await dialog.getByLabel('听成').fill('飞格玛');await dialog.getByLabel('改为').fill('Figma');await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(main.getByRole('cell',{name:'飞格玛',exact:true})).toBeVisible();
  const second=spawn(exe,[],{cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:id,SAYWEAVE_SECOND_INSTANCE_ACCEPTANCE:'1'},stdio:['ignore','pipe','pipe']});
  let secondError='';second.stderr.on('data',chunk=>{secondError+=chunk;});
  const secondExit=await Promise.race([new Promise(resolve=>second.once('exit',resolve)),delay(8000).then(()=>{second.kill();throw Error('Second instance did not exit');})]);
  report.secondInstance={exit:secondExit,stderr:secondError};
  expect(secondExit).toBe(0);expect(secondError).toContain('SAYWEAVE_ALREADY_RUNNING');report.checks.push('second real process refused same data directory');
  const overlay=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('surface=overlay'));if(!overlay)throw Error('Overlay unavailable');
  const denied=await overlay.evaluate(async()=>{try{await window.__TAURI_INTERNALS__.invoke('dictionary_list',{query:{kind:'term',search:'',page:1}});return null;}catch(error){return error;}});expect(denied).toMatchObject({code:'forbidden'});report.checks.push('actual overlay IPC denied');
  for(let i=0;i<50;i++){const at=performance.now();await invoke(main,'dictionary_list',{query:{kind:'term',search:'',page:1}});report.ipc.push(performance.now()-at);}
  report.viewport=await main.evaluate(()=>({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,overflow:document.documentElement.scrollWidth>innerWidth}));expect(report.viewport.overflow).toBe(false);
  const green=await main.locator('body *').evaluateAll(nodes=>nodes.filter(n=>Array.from(n.childNodes).some(c=>c.nodeType===3&&c.textContent?.trim())&&getComputedStyle(n).color==='rgb(117, 251, 145)').length);expect(green).toBe(0);
  await main.screenshot({path:path.join(root,'tests','visual','dictionary-desktop.png')});report.checks.push('production local assets, neutral text and actual scaling');
  await stop();expect(existsSync(path.join(directory,'data','dictionary.sqlite'))).toBe(true);
  main=await start(2);await expect(main.getByRole('cell',{name:'Figma Desktop',exact:true})).toBeVisible();await main.getByRole('tab',{name:'常见纠错'}).click();await expect(main.getByRole('cell',{name:'飞格玛',exact:true})).toBeVisible();
  await main.getByRole('button',{name:'删除 飞格玛'}).click();dialog=main.getByRole('dialog');await dialog.getByRole('button',{name:'取消',exact:true}).click();await expect(main.getByRole('cell',{name:'飞格玛',exact:true})).toBeVisible();await main.getByRole('button',{name:'删除 飞格玛'}).click();await dialog.getByRole('button',{name:'删除',exact:true}).click();await expect(main.getByText('还没有词条')).toBeVisible();await stop();
  main=await start(3);await expect(main.getByRole('cell',{name:'Figma Desktop',exact:true})).toBeVisible();await main.getByRole('tab',{name:'常见纠错'}).click();await expect(main.getByText('还没有词条')).toBeVisible();report.checks.push('three real process launches: both types persist, deletion persists');
  report.ipc.sort((a,b)=>a-b);report.ipcP95Ms=report.ipc[47];report.ipcIncludes='Playwright evaluation, IPC, worker queue and SQLite; one persisted term';
  await stop();
  // Replace only this isolated, stopped test profile with the benchmark's verified snapshot.
  copyFileSync(path.join(root,'.local','dictionary-benchmark-10000.sqlite'),path.join(directory,'data','dictionary.sqlite'));
  main=await start(4);await expect(main.getByText('10000 条',{exact:true})).toBeVisible();
  const samples=[];for(let i=0;i<50;i++){const at=performance.now();await invoke(main,'dictionary_list',{query:{kind:'term',search:'figma',page:1}});samples.push(performance.now()-at);}
  samples.sort((a,b)=>a-b);
  const at=performance.now();await main.getByRole('textbox',{name:'搜索词条'}).fill('figma');await expect(main.getByText('1000 条',{exact:true})).toBeVisible();
  report.largeData={rows:10000,ipcP95Ms:samples[47],uiSearchMs:performance.now()-at,uiIncludes:'input, 180ms debounce, IPC, query and rendered count'};
  await stop();
  const metrics=[...readFileSync(path.join(directory,'round-4-errors.log'),'utf8').matchAll(/operation=list queue_us=(\d+) execution_us=(\d+)/g)].slice(-50);
  expect(metrics.length).toBe(50);const p95=index=>metrics.map(m=>Number(m[index])/1000).sort((a,b)=>a-b)[47];
  report.largeData.queueP95Ms=p95(1);report.largeData.executionP95Ms=p95(2);
  expect(report.largeData.ipcP95Ms).toBeLessThan(100);
  report.checks.push('10000-row actual desktop search: database, queue, IPC and rendered UI timings');report.passed=true;
  }
}catch(error){report.passed=false;report.error=String(error);process.exitCode=1;}finally{await stop();writeFileSync(path.join(root,'.local',regular?'dictionary-release-report.json':'dictionary-desktop-report.json'),JSON.stringify(report,null,2));}
console.log(JSON.stringify(report,null,2));
