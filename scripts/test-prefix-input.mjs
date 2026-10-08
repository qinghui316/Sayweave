// Owned Windows/Chromium targets; no ASR, supplier, personal data or clipboard.
import {chromium,expect} from '@playwright/test';
import {build} from 'vite';
import {spawn} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const folder=path.join(root,'.local','prefix-input',randomUUID());mkdirSync(folder,{recursive:true});
const shape=process.argv[3]??'selected',engine=process.argv[4]??'chromium';
const gap=process.argv[5]==='gap';
const stepMs=Number(process.env.SAYWEAVE_PREFIX_STEP_MS??(gap?1500:250));
if(!Number.isInteger(stepMs)||stepMs<100||stepMs>2000)throw Error('Invalid owned prefix step interval');
const beginMs=Number(process.env.SAYWEAVE_PREFIX_BEGIN_MS??stepMs),firstMs=Number(process.env.SAYWEAVE_PREFIX_FIRST_MS??stepMs);
if([beginMs,firstMs].some(ms=>!Number.isInteger(ms)||ms<0||ms>2000))throw Error('Invalid owned stage interval');
const renderPauseMs=Number(process.env.SAYWEAVE_PREFIX_RENDER_PAUSE_MS??0);
if(!Number.isInteger(renderPauseMs)||renderPauseMs<0||renderPauseMs>1000)throw Error('Invalid owned renderer pause');
const renderPauseEvent=process.env.SAYWEAVE_PREFIX_RENDER_EVENT??'compositionstart';
if(!['compositionstart','beforeinput','compositionend'].includes(renderPauseEvent))throw Error('Invalid owned pause event');
const setup=process.env.SAYWEAVE_PREFIX_SETUP??'state';
if(!['state','keyboard'].includes(setup))throw Error('Invalid owned selection setup');
const returnFocus=process.env.SAYWEAVE_PREFIX_RETURN_FOCUS==='1';
const reselect=process.env.SAYWEAVE_PREFIX_RESELECT;
if(reselect&&(!['collapse','move','other'].includes(reselect)||process.argv[2]!=='ProseMirror-single'))throw Error('Invalid owned programmatic selection case');
const abortInitial=process.env.SAYWEAVE_PREFIX_ABORT_INITIAL;
if(abortInitial&&!['before','after'].includes(abortInitial))throw Error('Unknown initial cancellation stage');
const editPrepared=process.env.SAYWEAVE_PREFIX_EDIT_PREPARED;
if(editPrepared&&(!['text','caret'].includes(editPrepared)||shape!=='selected'||process.argv[2]!=='ProseMirror-single'||abortInitial))throw Error('Invalid prepared selection edit case');
if(process.argv[5]&&!gap)throw Error('Unknown prefix continuation mode');
const interference=process.argv[6];
if(abortInitial&&interference)throw Error('Initial cancellation cannot be combined with later interference');
const suffixCase=['suffix','suffix-abort'].includes(interference);
const abortTail=interference==='suffix-abort';
if(interference&&(!gap||process.argv[2]!=='ProseMirror-single'||!['delete-prefix','replace-prefix','outside-edit','outside-delete','right-edit','caret-return','suffix','suffix-abort'].includes(interference)))throw Error('Invalid owned interference scenario');
if(!['selected','selected-unicode','selected-second-line','selected-across-lines','empty','left','right'].includes(shape)||!['chromium','edge'].includes(engine))throw Error('Unknown fixture shape/runtime');
const report={passed:false,shape,engine,gap,stepMs,beginMs,firstMs,renderPauseMs,renderPauseEvent,setup,returnFocus,reselect,abortInitial,editPrepared,interference,cases:[],cleanup:false,nativeProtocol:Number(readFileSync(path.join(root,'src-tauri/text-service/src/wire.rs'),'utf8').match(/PROTOCOL: u32 = (\d+)/)[1])};
const wait=ms=>new Promise(resolve=>setTimeout(resolve,ms));
let browser,helper,probe,imeTraceSession;
const imeTrace=[];
const traceState={complete:false,truncated:false};
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
 if(process.env.SAYWEAVE_PREFIX_TRACE_IME==='1'){
  imeTraceSession=await browser.newBrowserCDPSession();
  imeTraceSession.on('Tracing.dataCollected',({value})=>{
   const events=value.filter(item=>item.cat?.split(',').includes('ime'));
   const remaining=10000-imeTrace.length;
   if(events.length>remaining)traceState.truncated=true;
   imeTrace.push(...events.slice(0,remaining));
  });
  await imeTraceSession.send('Tracing.start',{categories:'ime',transferMode:'ReportEvents'});
 }
 const session=await browser.newBrowserCDPSession();
 const pid=(await session.send('SystemInfo.getProcessInfo')).processInfo.find(item=>item.type==='browser').id;await session.detach();
 const page=await browser.newPage();
 await page.setContent('<input id="input"><textarea id="area"></textarea><div id="editable" contenteditable="true"></div>');
 const bundle=await build({configFile:false,logLevel:'error',build:{write:false,lib:{entry:path.join(root,'tests/fixtures/input-target.ts'),name:'InputFixture',formats:['iife']}}});
 await page.addScriptTag({content:(Array.isArray(bundle)?bundle[0]:bundle).output[0].code});
 const cases=[...['input','area','editable','ProseMirror'].map(control=>({control,multiline:false})),...['area','editable','ProseMirror','ProseMirrorInline'].map(control=>({control,multiline:true}))];
 const selected=process.argv[2]==='all'?null:process.argv[2];
 if(selected&&!cases.some(item=>`${item.control}-${item.multiline?'multiline':'single'}`===selected))throw Error('Unknown owned prefix case');
 const selectedCases=cases.filter(item=>(!gap||!item.multiline)&&(!selected||`${item.control}-${item.multiline?'multiline':'single'}`===selected));
 if(!selectedCases.length)throw Error('No matching owned prefix cases');
 for(const {control,multiline} of selectedCases){
  const rich=control.startsWith('ProseMirror');let binding=control==='ProseMirrorInline'?'fixtureInlineEditor':'fixtureEditor';
  const sample={selected:{before:'前被选右',start:1,end:3},'selected-unicode':{before:'前🙂被选\n右',start:1,end:5},'selected-second-line':{before:'前\n🙂被选右',start:2,end:6},'selected-across-lines':{before:'前被\n选右',start:1,end:4},empty:{before:'',start:0,end:0},left:{before:'前',start:1,end:1},right:{before:'右',start:0,end:0}}[shape];
  const before=sample.before;let startOffset=sample.start,endOffset=sample.end,left=before.slice(0,startOffset),right=before.slice(endOffset);
  if(setup==='keyboard'){
   const target=rich?page.locator(control==='ProseMirrorInline'?'#prosemirror-inline .ProseMirror':'#prosemirror .ProseMirror'):page.locator('#'+control);
   await page.bringToFront();
   const foreground=await focus({action:'focus-process',process:pid});if(foreground.foregroundProcess!==pid)throw Error('Owned browser did not receive foreground before selection');
   await target.focus();
  }
  // Only this synthetic fixture's generated marked breaks are decoded live.
  if([before,'ASR 第一段，第二段临时','第一段。','第二段。','尾部完成🙂'].some(text=>text.includes('\u2028')))throw Error('Literal U+2028 requires position-aware fixture assertions');
  if(rich)await page.evaluate(({sample,binding})=>window[binding].set(sample.before,sample.start,sample.end),{sample:{before,start:setup==='keyboard'?endOffset:startOffset,end:endOffset},binding});
  else await page.locator('#'+control).evaluate((node,sample)=>{if('value'in node){node.value=sample.before;node.focus();node.setSelectionRange(sample.start,sample.end);}else{node.textContent=sample.before;node.focus();const selection=getSelection(),range=document.createRange();range.setStart(node.firstChild??node,sample.start);range.setEnd(node.firstChild??node,sample.end);selection.removeAllRanges();selection.addRange(range);}},{before,start:setup==='keyboard'?endOffset:startOffset,end:endOffset});
  await page.bringToFront();
  if(renderPauseMs)await page.evaluate(({ms,event})=>document.addEventListener(event,()=>{const end=performance.now()+ms;while(performance.now()<end){/* Owned renderer stall for ordering acceptance. */}},{once:true}),{ms:renderPauseMs,event:renderPauseEvent});
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
  if(setup==='keyboard'&&startOffset!==endOffset){
   const target=rich?page.locator(control==='ProseMirrorInline'?'#prosemirror-inline .ProseMirror':'#prosemirror .ProseMirror'):page.locator('#'+control);
   const selectionSteps=[...new Intl.Segmenter().segment(before.slice(startOffset,endOffset))].length;
   for(let index=0;index<selectionSteps;index++)await target.press('Shift+ArrowLeft');
   await expect.poll(()=>rich?page.evaluate(binding=>window[binding].inspect().semanticSelection,binding):target.evaluate(node=>'selectionStart'in node?{start:node.selectionStart,end:node.selectionEnd}:{start:getSelection().getRangeAt(0).startOffset,end:getSelection().getRangeAt(0).endOffset}),{message:'Owned keyboard selection precondition',timeout:2000}).toEqual({start:startOffset,end:endOffset});
  }
  if(returnFocus){
   const readSelection=()=>rich?page.evaluate(binding=>window[binding].inspect().selection,binding):page.locator('#'+control).evaluate(node=>'selectionStart'in node?{from:node.selectionStart,to:node.selectionEnd}:{from:getSelection().getRangeAt(0).startOffset,to:getSelection().getRangeAt(0).endOffset});
   const before=await readSelection();
   const left=await focus({action:'focus'});
   const transition={before,expectedAwayProcess:helper.pid,awayProcess:left.foregroundProcess,expectedReturnProcess:pid};
   (report.focusTransitions??=[]).push(transition);
   if(left.foregroundProcess!==helper.pid)throw Error('Owned focus did not leave browser');
   await wait(100);
   const restored=await focus({action:'focus-process',process:pid});transition.returnProcess=restored.foregroundProcess;
   if(restored.foregroundProcess!==pid)throw Error('Owned focus return failed');
   transition.after=await readSelection();
   if(transition.after.from!==before.from||transition.after.to!==before.to)throw Error('Owned focus return did not preserve selection');
  }
  let output='',finished=false,code;
  if(reselect){
   if(reselect==='collapse')startOffset=endOffset;
   else if(reselect==='move'){startOffset=0;endOffset=1;}
   else{
    binding='fixtureInlineEditor';
    await page.evaluate(({before,start,end})=>window.fixtureInlineEditor.set(before,start,end),{before,start:startOffset,end:endOffset});
   }
   await page.evaluate(({binding,start,end})=>window[binding].select(start,end),{binding,start:startOffset,end:endOffset});
   left=before.slice(0,startOffset);right=before.slice(endOffset);
  }
  if(process.env.SAYWEAVE_PREFIX_READ_SELECTION==='1'){
   const evidence=await focus({action:'inspect-selection',process:pid});
   (report.selectionEvidence??=[]).push({control,...evidence});
  }
  probe=start(path.join(root,'src-tauri/text-service/target/debug/tsf-probe.exe'),['probe',String(pid),'0',abortInitial?`prefix-abort-${abortInitial}`:abortTail?'prefix-gap-abort':gap?'prefix-gap':multiline?'prefix-multiline':'prefix',String(stepMs),String(beginMs),String(firstMs)],{SAYWEAVE_INPUT_ACCEPTANCE:'1'});
  probe.stdout.on('data',b=>{output+=b.toString('utf8');});probe.stderr.on('data',b=>{output+=b.toString('utf8');});
  probe.on('error',error=>{output+=error.message;code=-1;finished=true;});probe.on('exit',exit=>{code=exit;finished=true;});
  const read=async()=>rich?page.evaluate(binding=>window[binding].read(),binding):page.locator('#'+control).evaluate(node=>'value'in node?node.value:node.innerText);
  const observed=new Set(),deadline=Date.now()+18000;
  let interfered=false,protectedText;
  while(!finished){
   observed.add((await read()).replace(/\r\n?/g,'\n').replace(/\u2028/g,'\n'));
   if(editPrepared&&!interfered&&output.includes('operation=2 revision=0 applied=true')){
    if(output.includes('operation=3'))throw Error('Prepared edit missed the first-write boundary');
    interfered=true;
    await page.evaluate(mode=>{if(mode==='text')window.fixtureEditor.edit(1,3,'用户');else window.fixtureEditor.select(0);},editPrepared);
    protectedText=await read();
    if(protectedText!==(editPrepared==='text'?'前用户右':before))throw Error('Prepared user edit did not apply');
   }
   if(interference&&!interfered&&output.includes('operation=4 revision=2 applied=true')){
    interfered=true;await wait(350);
    const settled=await read();
    if(settled!==`${left}第一段。${right}`)throw Error('Interference requires the fully settled first prefix');
    observed.add(settled);
    if(suffixCase){
     await page.evaluate(()=>window.fixtureEditor.appendParagraph());
    }else if(interference==='right-edit'){
     if(!right)throw Error('Right edit requires right context');
     await page.evaluate(position=>window.fixtureEditor.edit(position,position+1,'改'),left.length+4);
    }else if(interference==='caret-return'){
     await page.evaluate(()=>window.fixtureEditor.select(0));await wait(150);
     const moved=await page.evaluate(()=>window.fixtureEditor.inspect());
     if(moved.selection.from!==1||moved.selection.to!==1)throw Error('Caret did not leave the settled position');
     await page.evaluate(position=>window.fixtureEditor.select(position),left.length+4);
    }else{
     if(interference.startsWith('outside-')&&!left)throw Error('Outside edit requires left context');
     await page.evaluate(({mode,left})=>{
      const from=mode.startsWith('outside-')?0:left.length;
      window.fixtureEditor.edit(from,from+(mode==='delete-prefix'?4:1),mode.endsWith('delete')||mode==='delete-prefix'?'':'改');
     },{mode:interference,left});
    }
    protectedText=await read();
    const expectedEdit=suffixCase?`${left}第一段。${right}\n`:interference==='right-edit'?`${left}第一段。改${right.slice(1)}`:interference==='delete-prefix'?`${left}${right}`:interference==='replace-prefix'?`${left}改一段。${right}`:interference==='outside-edit'?`改${left.slice(1)}第一段。${right}`:interference==='outside-delete'?`${left.slice(1)}第一段。${right}`:`${left}第一段。${right}`;
    if(protectedText!==expectedEdit)throw Error('Owned edit did not produce its expected text');
   }
   if(Date.now()>deadline){probe.kill();throw Error('Owned prefix probe timeout');}await wait(25);
  }
  probe=null;writeFileSync(path.join(folder,control+(multiline?'-multiline':'-single')+'.log'),output);
  const actual=(await read()).replace(/\r\n?/g,'\n');
  const line=multiline?'\n':'';
  const expected=editPrepared?protectedText:abortInitial?before:abortTail?`${left}第一段。${right}\n`:`${left}第一段。${line}第二段。${line}尾部完成🙂${right}${suffixCase?'\n':''}`;
  const liveAsr=observed.has(`${left}ASR 第一段${gap?'':'，第二段临时'}${right}`);
  const firstPrefix=observed.has(`${left}第一段。${line}${gap?'':'第二段临时'}${right}`);
  const growingTail=observed.has(`${left}第一段。${line}第二段续写🙂${right}${suffixCase?'\n':''}`);
  const secondPrefix=observed.has(`${left}第一段。${line}第二段。${line}${gap?'':'尾部🙂'}${right}${suffixCase?'\n':''}`);
  const finalDom=rich?await page.evaluate(binding=>window[binding].inspect(),binding):undefined;
  if(reselect==='other'){
   report.originalEditorPreserved=await page.evaluate(()=>window.fixtureEditor.read())===before;
   if(!report.originalEditorPreserved)throw Error('Original editor was modified after switching to identical text');
  }
  const screenshot=path.join(folder,control+(multiline?'-multiline':'-single')+'.png');await page.screenshot({path:screenshot});
  const events=rich&&actual!==expected?await page.evaluate(()=>window.ownedPrefixEvents):undefined;
  const focusEvidence=code!==0&&/foreground/.test(output)?await focus({action:'read'}):undefined;
  report.cases.push({control,multiline,code,liveAsr,firstPrefix,growingTail,secondPrefix,actualTextCorrect:actual===expected,screenshot,finalDom,...(focusEvidence?{focusEvidence:{expectedProcess:pid,actualProcess:focusEvidence.foregroundProcess}}:{}),...(actual!==expected?{ownedFixtureActual:actual,ownedFixtureExpected:expected,ownedFixtureSnapshots:[...observed],ownedFixtureEvents:events}:{})});
  if(editPrepared){
   report.cases.at(-1).editProtected=interfered&&code!==0&&/operation=3 revision=1 applied=false/.test(output)&&actual===protectedText&&!liveAsr;
   if(!report.cases.at(-1).editProtected)throw Error('Prepared user edit was overwritten');
  }else if(abortInitial){
   const restored=rich?finalDom.semanticSelection:await page.locator('#'+control).evaluate(node=>'selectionStart'in node?{start:node.selectionStart,end:node.selectionEnd}:{start:getSelection().getRangeAt(0).startOffset,end:getSelection().getRangeAt(0).endOffset});
   const selectionRestored=restored.start===startOffset&&restored.end===endOffset;
   const caretAtRestoredEnd=restored.start===endOffset&&restored.end===endOffset;
   Object.assign(report.cases.at(-1),{restoredSelection:restored,selectionRestored,caretAtRestoredEnd});
   if(code!==0||actual!==before||(!selectionRestored&&!caretAtRestoredEnd)||(abortInitial==='after'&&!liveAsr))throw Error('Initial cancellation did not restore selected text with a valid final caret');
  }else if(interference&&!suffixCase){
   const refused=code!==0&&/operation=5 revision=3 applied=false code=(draftOwnership|edit:(receiptPosition|position|selection|gapPosition))/.test(output);
   report.cases.at(-1).editProtected=interfered&&refused&&actual===protectedText;
   if(!report.cases.at(-1).editProtected)throw Error('Owned external edit was not protected');
  }else if(code!==0||!liveAsr||!firstPrefix||!growingTail||(!abortTail&&!secondPrefix)||actual!==expected||(suffixCase&&!interfered))throw Error('Owned '+control+' actual prefix/tail text assertions failed');
 }
 report.passed=true;
}catch(error){report.error=String(error);process.exitCode=1;}
finally{
 const errors=[];
 if(imeTraceSession){let timeout;try{
  const done=new Promise(resolve=>imeTraceSession.once('Tracing.tracingComplete',resolve));
  await Promise.race([(async()=>{await imeTraceSession.send('Tracing.end');await done;traceState.complete=true;})(),new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('Owned trace completion timeout')),3000);})]);
 }catch(error){errors.push('owned trace cleanup: '+String(error));}finally{
  clearTimeout(timeout);
  report.imeTrace={...traceState,eventCount:imeTrace.length};
  try{writeFileSync(path.join(folder,'owned-ime-trace.json'),JSON.stringify({...report.imeTrace,events:imeTrace},null,2));}catch(error){errors.push('owned trace save: '+String(error));}
  // Closing the owned browser below detaches the tracing session as well.
 }}
 if(browser){try{await browser.close();}catch(error){errors.push('browser: '+String(error));}}
 if(helper&&!exited(helper)){try{helper.stdin.end();await wait(200);}catch(error){errors.push('helper: '+String(error));}}
 for(const child of owned){try{if(!await stop(child))errors.push('owned child did not exit: '+child.pid);}catch(error){errors.push('owned child cleanup: '+String(error));}}
 report.cleanup=errors.length===0&&[...owned].every(exited);
 if(!report.cleanup){report.cleanupErrors=errors;report.passed=false;process.exitCode=1;}
 writeFileSync(path.join(folder,'report.json'),JSON.stringify(report,null,2));
 console.log(JSON.stringify({report:path.join(folder,'report.json'),...report,cases:report.cases.map(item=>{const copy={...item};delete copy.ownedFixtureEvents;return copy;})}));
}
