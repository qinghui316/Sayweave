// Owned Windows/Chromium targets; no ASR, supplier, personal data or clipboard.
import {chromium} from '@playwright/test';
import {build} from 'vite';
import {spawn} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const folder=path.join(root,'.local','prefix-input',randomUUID());mkdirSync(folder,{recursive:true});
const shape=process.argv[3]??'selected',engine=process.argv[4]??'chromium';
if(!['selected','empty','left','right'].includes(shape)||!['chromium','edge'].includes(engine))throw Error('Unknown fixture shape/runtime');
const report={passed:false,shape,engine,cases:[],cleanup:false,nativeProtocol:Number(readFileSync(path.join(root,'src-tauri/text-service/src/wire.rs'),'utf8').match(/PROTOCOL: u32 = (\d+)/)[1])};
const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
let browser,helper,probe;
const owned=new Set();
function start(file,args,env={}){const child=spawn(file,args,{cwd:root,windowsHide:true,env:{...process.env,...env},stdio:['pipe','pipe','pipe']});owned.add(child);return child;}
function exited(child){return child.exitCode!==null||child.signalCode!==null||child.pid===undefined;}
async function stop(child){if(exited(child))return true;const exit=new Promise(resolve=>child.once('exit',()=>resolve(true)));child.kill();return Promise.race([exit,wait(3000).then(()=>exited(child))]);}
async function command(file,args){const child=start(file,args);let output='';child.stdout.on('data',b=>{output+=b.toString('utf8');});child.stderr.on('data',b=>{output+=b.toString('utf8');});const code=await new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',resolve);});writeFileSync(path.join(folder,'build.log'),output,{flag:'a'});if(code!==0)throw Error('Owned fixture build failed');}
function requestFixture(child){let buffer='',pending;child.stdout.on('data',data=>{buffer+=data.toString('utf8');let index;while((index=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,index);buffer=buffer.slice(index+1);if(line&&pending){const resolve=pending;pending=null;resolve(JSON.parse(line));}}});return async value=>{if(pending)throw Error('One owned fixture request at a time');const response=new Promise(resolve=>{pending=resolve;});child.stdin.write(JSON.stringify(value)+'\n');const timeout=wait(5000).then(()=>{throw Error('Owned focus helper timeout');});return Promise.race([response,timeout]);};}
try{
 if(process.platform!=='win32')throw Error('Windows TSF prefix acceptance requires Windows');
 await command('cargo',['build','--manifest-path','src-tauri/text-service/Cargo.toml','--bin','tsf-probe']);
 await command('cargo',['build','--manifest-path','src-tauri/Cargo.toml','--features','desktop-acceptance','--bin','input-fixture']);
 helper=start(path.join(root,'src-tauri/target/debug/input-fixture.exe'),['composition'],{SAYWEAVE_INPUT_ACCEPTANCE:'1'});
 const focus=requestFixture(helper);await wait(400);
 browser=await chromium.launch({headless:false,...(engine==='edge'?{executablePath:'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'}:{}),args:['--force-renderer-accessibility']});
 report.browserVersion=browser.version();
 const session=await browser.newBrowserCDPSession();
 const pid=(await session.send('SystemInfo.getProcessInfo')).processInfo.find(item=>item.type==='browser').id;await session.detach();
 const page=await browser.newPage();
 await page.setContent('<input id="input"><textarea id="area"></textarea><div id="editable" contenteditable="true"></div>');
 const bundle=await build({configFile:false,logLevel:'error',build:{write:false,lib:{entry:path.join(root,'tests/fixtures/input-target.ts'),name:'InputFixture',formats:['iife']}}});
 await page.addScriptTag({content:(Array.isArray(bundle)?bundle[0]:bundle).output[0].code});
 const cases=[...['input','area','editable','ProseMirror'].map(control=>({control,multiline:false})),...['area','editable','ProseMirror','ProseMirrorInline'].map(control=>({control,multiline:true}))];
 const selected=process.argv[2]==='all'?null:process.argv[2];
 if(selected&&!cases.some(item=>`${item.control}-${item.multiline?'multiline':'single'}`===selected))throw Error('Unknown owned prefix case');
 for(const {control,multiline} of cases.filter(item=>!selected||`${item.control}-${item.multiline?'multiline':'single'}`===selected)){
  const rich=control.startsWith('ProseMirror'),binding=control==='ProseMirrorInline'?'fixtureInlineEditor':'fixtureEditor';
  const sample={selected:{before:'前被选右',start:1,end:3},empty:{before:'',start:0,end:0},left:{before:'前',start:1,end:1},right:{before:'右',start:0,end:0}}[shape];
  const before=sample.before,startOffset=sample.start,endOffset=sample.end,left=before.slice(0,startOffset),right=before.slice(endOffset);
  // Only this synthetic fixture's generated marked breaks are decoded live.
  if([before,'ASR 第一段，第二段临时','第一段。','第二段。','尾部完成🙂'].some(text=>text.includes('\u2028')))throw Error('Literal U+2028 requires position-aware fixture assertions');
  if(rich)await page.evaluate(({sample,binding})=>window[binding].set(sample.before,sample.start,sample.end),{sample:{before,start:startOffset,end:endOffset},binding});
  else await page.locator('#'+control).evaluate((node,sample)=>{if('value'in node){node.value=sample.before;node.focus();node.setSelectionRange(sample.start,sample.end);}else{node.textContent=sample.before;node.focus();const selection=getSelection(),range=document.createRange();range.setStart(node.firstChild??node,sample.start);range.setEnd(node.firstChild??node,sample.end);selection.removeAllRanges();selection.addRange(range);}},{before,start:startOffset,end:endOffset});
  await page.bringToFront();
  if(rich)await page.evaluate(binding=>{
   window.ownedPrefixEventController?.abort();
   window.ownedPrefixEventController=new AbortController();
   window.ownedPrefixEvents=[];
   for(const type of ['compositionstart','compositionupdate','compositionend','beforeinput','input'])document.addEventListener(type,event=>{
    if(!event.target.closest?.(binding==='fixtureInlineEditor'?'#prosemirror-inline':'#prosemirror'))return;
    const snapshot=()=>window.ownedPrefixEvents.push({type,eventData:event.data,inputType:event.inputType,phase:'after-event',state:window[binding].inspect()});
    window.ownedPrefixEvents.push({type,eventData:event.data,inputType:event.inputType,phase:'capture',state:window[binding].inspect()});
    queueMicrotask(snapshot);
   },{capture:true,signal:window.ownedPrefixEventController.signal});
  },binding);
  const foreground=await focus({action:'focus-process',process:pid});if(foreground.foregroundProcess!==pid)throw Error('Owned browser did not receive foreground');
  let output='',finished=false,code;
  probe=start(path.join(root,'src-tauri/text-service/target/debug/tsf-probe.exe'),['probe',String(pid),'0',multiline?'prefix-multiline':'prefix','250'],{SAYWEAVE_INPUT_ACCEPTANCE:'1'});
  probe.stdout.on('data',b=>{output+=b.toString('utf8');});probe.stderr.on('data',b=>{output+=b.toString('utf8');});
  probe.on('error',error=>{output+=error.message;code=-1;finished=true;});probe.on('exit',exit=>{code=exit;finished=true;});
  const read=async()=>rich?page.evaluate(binding=>window[binding].read(),binding):page.locator('#'+control).evaluate(node=>'value'in node?node.value:node.innerText);
  const observed=new Set(),deadline=Date.now()+18000;
  while(!finished){observed.add((await read()).replace(/\r\n?/g,'\n').replace(/\u2028/g,'\n'));if(Date.now()>deadline){probe.kill();throw Error('Owned prefix probe timeout');}await wait(25);}
  probe=null;writeFileSync(path.join(folder,control+(multiline?'-multiline':'-single')+'.log'),output);
  const actual=(await read()).replace(/\r\n?/g,'\n');
  const line=multiline?'\n':'';
  const expected=`${left}第一段。${line}第二段。${line}尾部完成🙂${right}`;
  const liveAsr=observed.has(`${left}ASR 第一段，第二段临时${right}`);
  const firstPrefix=observed.has(`${left}第一段。${line}第二段临时${right}`);
  const growingTail=observed.has(`${left}第一段。${line}第二段续写🙂${right}`);
  const secondPrefix=observed.has(`${left}第一段。${line}第二段。${line}尾部🙂${right}`);
  const finalDom=rich?await page.evaluate(binding=>window[binding].inspect(),binding):undefined;
  const screenshot=path.join(folder,control+(multiline?'-multiline':'-single')+'.png');await page.screenshot({path:screenshot});
  const events=rich&&actual!==expected?await page.evaluate(()=>window.ownedPrefixEvents):undefined;
  const focusEvidence=code!==0&&/foreground/.test(output)?await focus({action:'read'}):undefined;
  report.cases.push({control,multiline,code,liveAsr,firstPrefix,growingTail,secondPrefix,actualTextCorrect:actual===expected,screenshot,finalDom,...(focusEvidence?{focusEvidence:{expectedProcess:pid,actualProcess:focusEvidence.foregroundProcess}}:{}),...(actual!==expected?{ownedFixtureActual:actual,ownedFixtureExpected:expected,ownedFixtureSnapshots:[...observed],ownedFixtureEvents:events}:{})});
  if(code!==0||!liveAsr||!firstPrefix||!growingTail||!secondPrefix||actual!==expected)throw Error('Owned '+control+' actual prefix/tail text assertions failed');
 }
 report.passed=true;
}catch(error){report.error=String(error);process.exitCode=1;}
finally{
 const errors=[];
 if(browser){try{await browser.close();}catch(error){errors.push('browser: '+String(error));}}
 if(helper&&!exited(helper)){try{helper.stdin.end();await wait(200);}catch(error){errors.push('helper: '+String(error));}}
 for(const child of owned){try{if(!await stop(child))errors.push('owned child did not exit: '+child.pid);}catch(error){errors.push('owned child cleanup: '+String(error));}}
 report.cleanup=errors.length===0&&[...owned].every(exited);
 if(!report.cleanup){report.cleanupErrors=errors;report.passed=false;process.exitCode=1;}
 writeFileSync(path.join(folder,'report.json'),JSON.stringify(report,null,2));
 console.log(JSON.stringify({report:path.join(folder,'report.json'),...report,cases:report.cases.map(item=>{const copy={...item};delete copy.ownedFixtureEvents;return copy;})}));
}
