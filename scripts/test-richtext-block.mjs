// Scenario extension of test-prefix-input; all text belongs to its owned fixture.
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {writeFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';

// Bounded comparison through the installed native service and the actual IME.
// Only the owned editor is seeded programmatically; all subsequent typing uses
// guarded Win32 key injection, never DOM text replacement or CDP insertText.
export async function runPostDictationIme({page,pid,focus,start,folder,report}){
 const executable=process.env.SAYWEAVE_ACTOR_TEST_EXE;
 if(!executable||!path.isAbsolute(executable))throw Error('Built owned actor required');
 const repetitions=Number(process.env.SAYWEAVE_IME_REPETITIONS??10);
 if(!Number.isInteger(repetitions)||repetitions<1||repetitions>10)throw Error('IME comparison limit is 1..10');
 const modes=process.env.SAYWEAVE_IME_MODES?.split(',')??['direct','finish','stop'];
 if(!modes.length||new Set(modes).size!==modes.length||modes.some(mode=>!['direct','finish','stop'].includes(mode)))throw Error('Unknown IME comparison modes');
 const initialText=process.env.SAYWEAVE_IME_SAME_BODY==='1'?'块外｜下周一上午开会':'块外｜';
 if(initialText!=='块外｜'&&modes.some(mode=>mode!=='direct'))throw Error('Same-body control is direct typing only');
 const trace=[],trials=[];report.imeTrials=trials;report.imeRepetitions=repetitions;report.imeModes=modes;
 report.imeInitialText=initialText;
 await page.evaluate(()=>{
  window.ownedImeEvents=[];window.ownedImeComposing=false;window.ownedImeChanges=0;
  new MutationObserver(()=>window.ownedImeChanges++).observe(document.querySelector('#prosemirror'),{subtree:true,characterData:true,childList:true});
  for(const type of ['keydown','keyup','compositionstart','compositionupdate','compositionend','beforeinput','input'])document.addEventListener(type,event=>{
   if(!event.target.closest?.('#prosemirror'))return;
   if(type==='compositionstart')window.ownedImeComposing=true;
   if(type==='compositionend')window.ownedImeComposing=false;
   const record=phase=>window.ownedImeEvents.push({type,phase,key:event.key,code:event.code,data:event.data,inputType:event.inputType,text:window.fixtureEditor.read(),composing:window.ownedImeComposing});
   record('capture');queueMicrotask(()=>record('microtask'));setTimeout(()=>record('after-task'),0);
  },true);
 });
 const settled=async label=>{
  const value=await page.evaluate(async()=>{
   let previous='',stable=0;const deadline=performance.now()+2000,samples=[];
   while(performance.now()<deadline){
    await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
    const sample={text:window.fixtureEditor.read(),dom:window.fixtureEditor.inspect(),composing:window.ownedImeComposing,events:window.ownedImeEvents.length,changes:window.ownedImeChanges};
    const key=JSON.stringify(sample);stable=key===previous?stable+1:0;previous=key;samples.push(sample);
    if(stable>=2)return {...sample,samples};
   }
   throw Error('Owned IME host did not settle');
  });
  const foreground=await focus({action:'read'});
  if(foreground.foregroundProcess!==pid)throw Error('Owned IME foreground changed');
  trace.push({label,...value});return value;
 };
 const key=async(code,down)=>{
  const result=await focus({action:'fixture-key',process:pid,key:code,down});
  if(!result.keyResult?.accepted)throw Error('Owned native key was not dispatched: '+code);
 };
 const tap=async code=>{await key(code,true);await key(code,false);};
 const type=async text=>{for(const letter of text)await tap('Key'+letter.toUpperCase());};
 const chinese=async prefix=>{
  await tap('Escape');await settled('before IME mode probe');
  for(let attempt=0;attempt<2;attempt++){
   await tap('KeyQ');const probe=await settled('IME mode probe');
   if(probe.composing){await tap('Escape');const cleared=await settled('cancel owned mode probe');if(cleared.text!==prefix)throw Error('IME probe cancellation changed committed body');return;}
   if(probe.text!==prefix+'q')throw Error('Unable to establish IME mode from owned events');
   await tap('Backspace');await settled('remove owned probe');await tap('ShiftLeft');
  }
  throw Error('A Chinese IME composition was not observed');
 };
 const dictate=async mode=>{
  const actor=start(executable,['--exact','dictation::drafts::tests::owned::richtext_actor','--ignored','--nocapture','--test-threads=1'],{SAYWEAVE_INPUT_ACCEPTANCE:'1',SAYWEAVE_TSF_OWNED_PID:String(pid),SAYWEAVE_DRAFT_STRATEGY:'revisable'});
  let buffer='',diagnostics='',exited=false,exitResult;const values=[],pending=[];
  actor.stderr.on('data',data=>diagnostics+=data.toString('utf8'));
  actor.stdout.on('data',data=>{buffer+=data.toString('utf8');let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at);buffer=buffer.slice(at+1);const index=line.indexOf('ACTOR_JSON:');if(index<0){diagnostics+=line+'\n';continue;}const value=JSON.parse(line.slice(index+11));if(pending.length)pending.shift().resolve(value);else values.push(value);}});
  const exit=new Promise(resolve=>actor.once('exit',(code,signal)=>{exited=true;exitResult={code,signal};for(const item of pending.splice(0))item.reject(Error('Actor exit '+code+': '+diagnostics));resolve(exitResult);}));
  const retirement=async()=>{let timer;try{return await Promise.race([exit,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Owned IME actor retirement timeout')),5500);})]);}finally{clearTimeout(timer);}};
  const receive=()=>values.length?Promise.resolve(values.shift()):new Promise((resolve,reject)=>{
   if(exited){reject(Error(diagnostics));return;}
   const item={resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}};
   const timer=setTimeout(()=>{const at=pending.indexOf(item);if(at>=0)pending.splice(at,1);reject(Error('Owned IME actor timeout'));},18000);pending.push(item);
  });
  const command=async value=>{actor.stdin.write(JSON.stringify(value)+'\n');return receive();};
  try{
   const identity=await receive();trace.push({label:'actor identity',...identity});
   const source={id:1,revision:1,text:'周',confirmed:false};
   for(const text of ['周','周三','周三开会']){
    source.text=text;source.revision++;await command({action:'asr',segments:[source]});
    const deadline=Date.now()+5000;let actual;
    do{actual=await settled('first native stream '+text);if(actual.text==='块外｜'+text)break;await command({action:'status'});}while(Date.now()<deadline);
    if(actual.text!=='块外｜'+text)throw Error('First stream actual body differs');
   }
   const result=await command({action:mode==='stop'?'stop':'finish'});trace.push({label:'actor terminal',...result});
   if(!result.finished||!result.delivery||result.delivery.restoreError||result.delivery.submission==='uncertain'||(result.error&&!(mode==='stop'&&result.error==='cancelled')))throw Error('Owned IME actor terminal was not safely settled');
   const failure=result.delivery.error;
   if(failure&&!(failure==='inputUnconfirmed'&&result.delivery.submission==='submitted')&&!(mode==='stop'&&failure==='cancelled'))throw Error('Owned IME actor delivery failed: '+failure);
   actor.stdin.end();
   const status=await retirement();trace.push({label:'actor exit',...status});
   if(status.code!==0||status.signal)throw Error('Owned IME actor exit failed: '+JSON.stringify(status));
   if(!diagnostics.includes('phase=resourcesReleased result=complete'))throw Error('Native retirement was not confirmed before IME typing');
   return (await settled('after native retirement')).text;
  }finally{
   if(!exited){actor.stdin.end();await retirement().catch(()=>{});}
   trace.push({label:'actor diagnostics',diagnostics,exit:exitResult??null});
  }
 };
 try{
  for(const mode of modes)for(let round=0;round<repetitions;round++)for(const sequence of ['letters-then-shift','shift-then-letters']){
   const trial={mode,round,sequence};trials.push(trial);
   await page.evaluate(text=>window.fixtureEditor.set(text,text.length,text.length),initialText);await page.bringToFront();
   if((await focus({action:'focus-process',process:pid})).foregroundProcess!==pid)throw Error('Owned IME target not foreground');
   await chinese(initialText);
   if(!report.imeModules){
    const command=`(Get-Process -Id ${pid}).Modules | Where-Object {$_.FileName -match 'DoubaoIME|tsf-oime'} | Select-Object -ExpandProperty FileName | ConvertTo-Json`;
    const raw=execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',command],{encoding:'utf8',windowsHide:true});
    report.imeModules=JSON.parse(raw||'[]');
    if(!JSON.stringify(report.imeModules).includes('DoubaoIME'))throw Error('The owned browser did not load Doubao IME');
   }
   // Establish mode before dictation. A probe/escape after completion could
   // itself clear the very handoff residue this comparison is observing.
   let prefix=initialText;if(mode!=='direct')prefix=await dictate(mode);
   if(sequence==='shift-then-letters')await tap('ShiftLeft');
   await type('hello');trial.first=await settled('English first token');
   if(sequence==='letters-then-shift')await tap('ShiftLeft');
   trial.shifted=await settled('English after Shift');
   await type('world');trial.continued=await settled('English continued');
   if(!trial.shifted.text.startsWith(prefix+'hello')||!trial.continued.text.startsWith(prefix+'hello')){
    report.firstDivergence={kind:'committedEnglishLost',mode,round,sequence,prefix,afterShift:trial.shifted.text,continued:trial.continued.text};
    throw Error('English prefix was lost through the real IME key path');
   }
   if(trial.continued.composing)await tap('ShiftLeft');
   trial.final=await settled('English final');
   if(trial.final.text!==prefix+'helloworld')throw Error('Final IME body differs');
   trial.passed=true;
  }
  report.imePassed=true;report.naturalTerminationReproduced=false;
 }finally{
  report.imeEventCount=await page.evaluate(()=>window.ownedImeEvents.length);
  writeFileSync(path.join(folder,'ime-trace.json'),JSON.stringify({trace,trials,events:await page.evaluate(()=>window.ownedImeEvents),firstDivergence:report.firstDivergence},null,2));
 }
}

// Existing Win32 fixture exposes its host-owned TSF termination operation.
// The same production SessionDraft/adapter/local model actor consumes updates.
export async function runOwnedTermination({focus,start,report,pid}){
 const executable=process.env.SAYWEAVE_ACTOR_TEST_EXE;
 if(!executable||!path.isAbsolute(executable))throw Error('Built actor required');
 const initial=await focus({action:'focus',text:'',start:0,end:0});
 if(!initial.foreground||initial.foregroundProcess!==pid)throw Error('Owned editor focus required');
 const actor=start(executable,['--exact','dictation::drafts::tests::owned::richtext_actor','--ignored','--nocapture','--test-threads=1'],{SAYWEAVE_INPUT_ACCEPTANCE:'1',SAYWEAVE_TSF_OWNED_PID:String(pid),SAYWEAVE_DRAFT_STRATEGY:'revisable'});
 let buffer='',diagnostics='',ended=false;const queue=[],waiting=[];
 actor.stderr.on('data',data=>diagnostics+=data.toString('utf8'));
 actor.stdout.on('data',data=>{buffer+=data.toString('utf8');let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at);buffer=buffer.slice(at+1);const index=line.indexOf('ACTOR_JSON:');if(index<0){diagnostics+=line+'\n';continue;}const value=JSON.parse(line.slice(index+11));if(waiting.length)waiting.shift().resolve(value);else queue.push(value);}});
 actor.once('exit',code=>{ended=true;for(const item of waiting.splice(0))item.reject(Error('Actor exit '+code+': '+diagnostics));});
 const receive=()=>queue.length?Promise.resolve(queue.shift()):new Promise((resolve,reject)=>{if(ended){reject(Error(diagnostics));return;}const item={resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}};const timer=setTimeout(()=>{waiting.splice(waiting.indexOf(item),1);reject(Error('Actor timeout: '+diagnostics));},18000);waiting.push(item);});
 const command=async value=>{actor.stdin.write(JSON.stringify(value)+'\n');return receive();};
 const trace=[];report.terminationTrace=trace;
 const expectBody=async(label,text)=>{
  const deadline=Date.now()+5000;let consecutive=0;
  while(Date.now()<deadline){
   const read=await focus({action:'read'});trace.push({label,...read});
   if(!read.foreground)throw Error('Owned editor focus lost');
   consecutive=read.text===text?consecutive+1:0;
   if(consecutive===3)return read;
   await new Promise(resolve=>setTimeout(resolve,30));
  }
  throw Error(label+': actual host body differs, expected '+text);
 };
 try{
  report.actorIdentity=await receive();
  const source={id:1,revision:1,text:'周三开会',confirmed:false};
  await command({action:'asr',segments:[source]});await expectBody('first write','周三开会');
  const deadline=Date.now()+5000;
  while(!diagnostics.includes('operation=draft_block_bound')){if(Date.now()>deadline)throw Error('Tracking not bound');await new Promise(resolve=>setTimeout(resolve,30));}
  const terminated=await focus({action:'terminate-composition'});trace.push({label:'host terminate',...terminated});
  if(!terminated.terminated)throw Error('Host termination failed '+JSON.stringify(terminated));
  await expectBody('after host events','周三开会');
  source.revision++;source.text='周三开会，不对，改成下周一上午开会';source.confirmed=true;
  await command({action:'asr',segments:[source]});
  let state;const detachedDeadline=Date.now()+5000;
  do{
   await new Promise(resolve=>setTimeout(resolve,30));state=await command({action:'status'});
  }while(!state.resultOnly&&Date.now()<detachedDeadline);
  trace.push({label:'delivery detached',...state});
  if(!state.resultOnly||state.presentation!=='recording'||state.audioRoute!==1)throw Error('Input refusal did not retain the recording route');
  if(!/code=draftOwnershipTerminated:[^\n]+mutation=NotAttempted/.test(diagnostics))throw Error('Exact no-write termination receipt not observed');
  if(diagnostics.includes('operation=draft_reacquire')||diagnostics.includes('operation=draft_reanchor'))throw Error('Unexpected automatic recovery after host termination');
  await expectBody('after delivery detach and host events','周三开会');
  source.revision++;source.text+='，再补充一个测试';
  await command({action:'asr',segments:[source]});
  await expectBody('later recognition cannot replay into the old field','周三开会');
  const result=await command({action:'finish'});trace.push({label:'explicit finish',...result});
  if(!result.finished||!result.resultOnly||result.resultRaw!==source.text)throw Error('Explicit finish lost the complete recording result');
  await expectBody('after finish and host events','周三开会');
  report.terminationDelivery={injected:true,recordingRetained:true,retained:'周三开会',result:result.resultRaw,native:result.delivery,naturalCauseProven:false};
 }finally{
  if(!ended){actor.stdin.end();await new Promise(resolve=>{const timer=setTimeout(resolve,5500);actor.once('exit',()=>{clearTimeout(timer);resolve();});});}
  report.actorDiagnostics=diagnostics;
 }
}

export async function runBlockRegression({page,pid,focus,start,root,folder,report}){
 const control=process.env.SAYWEAVE_BLOCK_CONTROL??'richtext';
 if(!['richtext','controlled-input'].includes(control))throw Error('Unknown owned block control');
 report.blockControl=control;
 if(control==='controlled-input'){
  if(process.env.SAYWEAVE_BLOCK_USER_NEWLINE==='1'||process.env.SAYWEAVE_BLOCK_FLOATING_PREDICTION==='1'||process.env.SAYWEAVE_BLOCK_BOUNDARIES==='1')throw Error('This scenario requires the rich-text host');
  await page.evaluate(()=>window.fixtureControlledInput());
 }
 const baselineMode=process.env.SAYWEAVE_BLOCK_BASELINE??'semantic-control';
 if(!['semantic-control','native-readback'].includes(baselineMode))throw Error('Unknown owned baseline mode');
 report.baselineMode=baselineMode;
 const trace=[];report.blockTrace=trace;
 await page.evaluate(()=>window.fixtureEditor.set('',0,0));
 await page.bringToFront();
 if((await focus({action:'focus-process',process:pid})).foregroundProcess!==pid)throw Error('Owned block foreground differs');
 await page.evaluate(()=>{
  window.ownedBlockEvents=[];window.ownedBlockChanges=0;
  new MutationObserver(()=>window.ownedBlockChanges++).observe(document.querySelector('#prosemirror'),{subtree:true,characterData:true,childList:true});
  for(const type of ['compositionstart','compositionupdate','compositionend','beforeinput','input'])document.addEventListener(type,event=>{
   if(!event.target.closest?.('#prosemirror'))return;
   const record=phase=>window.ownedBlockEvents.push({type,phase,data:event.data,inputType:event.inputType,body:window.fixtureEditor.read(),dom:window.fixtureEditor.inspect()});
   record('capture');queueMicrotask(()=>record('microtask'));setTimeout(()=>record('after-task'),0);
  },true);
 });
 const child=start(path.join(root,'src-tauri/text-service/target/debug/tsf-probe.exe'),['richtext-block-script',String(pid)],{SAYWEAVE_INPUT_ACCEPTANCE:'1',SAYWEAVE_TSF_OWNED_PID:String(pid)});
 const queue=[],pending=[];let buffer='',errors='',ended=false;
 child.stdout.on('data',data=>{buffer+=data.toString('utf8');let i;while((i=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,i);buffer=buffer.slice(i+1);if(!line.trim())continue;try{const value=JSON.parse(line);if(pending.length)pending.shift().resolve(value);else queue.push(value);}catch(error){for(const item of pending.splice(0))item.reject(error);}}});
 child.stderr.on('data',data=>{errors+=data.toString('utf8');});
 child.once('exit',code=>{ended=true;for(const item of pending.splice(0))item.reject(Error('Owned block probe exited '+code+': '+errors));});
 const next=()=>queue.length?Promise.resolve(queue.shift()):new Promise((resolve,reject)=>{if(ended){reject(Error(errors));return;}const item={resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}};const timer=setTimeout(()=>{const i=pending.indexOf(item);if(i>=0)pending.splice(i,1);reject(Error('Owned block response timeout: '+errors));},18000);pending.push(item);});
 const read=()=>page.evaluate(()=>({text:window.fixtureEditor.read(),dom:window.fixtureEditor.inspect()}));
 const settled=async()=>{
  const result=await page.evaluate(async()=>{
  const samples=[];const deadline=performance.now()+2000;let previous='',stable=0;
  while(performance.now()<deadline){
   await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
   await new Promise(resolve=>{const channel=new MessageChannel();channel.port1.onmessage=()=>{channel.port1.close();channel.port2.close();resolve();};channel.port2.postMessage(0);});
   const sample={text:window.fixtureEditor.read(),dom:window.fixtureEditor.inspect(),eventCount:window.ownedBlockEvents.length,changes:window.ownedBlockChanges};
   const key=JSON.stringify(sample);samples.push(sample);stable=key===previous?stable+1:0;previous=key;
   if(stable>=2)return {...sample,samples};
  }
   throw Error('Owned editor did not settle within the existing bounded read');
  });
  if(control==='controlled-input'&&result.dom.controlledValue!==result.text){
   report.firstDivergence={label:'controlled input after host events',kind:'controlledValue',actual:result.text,controlledValue:result.dom.controlledValue};
   throw Error('Rendered input and controlled React value differ after host events');
  }
  return result;
 };
 let revision=0,turn=1,snapshot,lastBody;
 const step=async(label,action,text='',block,requireApplied=true)=>{
  const before=await read();if(lastBody!==undefined&&before.text!==lastBody)throw Error('Late host body change before '+label);
  child.stdin.write(JSON.stringify({action,text,block,turn,revision})+'\n');
  const native=await next();const immediate=await read();const afterEvents=await settled();
  const row={label,before,native,immediate,settled:afterEvents};trace.push(row);lastBody=afterEvents.text;
  if(requireApplied&&!native.reply.applied){report.firstDivergence={label,kind:'nativeRefusal',code:native.reply.code};throw Error(label+': '+native.reply.code);}
  if(native.reply.block)snapshot=native.reply.block;
  return row;
 };
 const command=action=>({lease:snapshot.lease,id:snapshot.id,action,epoch:snapshot.epoch,version:snapshot.version,expected:snapshot.text,selectionStart:snapshot.selectionStart,selectionEnd:snapshot.selectionEnd});
 const check=(row,expected)=>{if(row.settled.text!==expected){report.firstDivergence={label:row.label,kind:'body',expected,actual:row.settled.text};throw Error(row.label+': actual rich-text body differs');}};
 try{
  report.executionIdentity=await next();
  if(process.env.SAYWEAVE_BLOCK_ACTOR_ONLY==='1'){
   if(process.env.SAYWEAVE_BLOCK_BOUNDARIES==='1'){
    report.boundaryCases=[];
    const cases=['outside-suffix','range-drift','document-rebuild'];
    const selected=process.env.SAYWEAVE_BLOCK_BOUNDARY_CASE;
    if(selected&&!cases.includes(selected))throw Error('Unknown boundary case');
    for(const mode of selected?[selected]:cases){
     await page.evaluate(()=>window.fixtureEditor.set('左|#右',2,2));
     lastBody=(await settled()).text;revision=0;
     await step(mode+' inspect','inspect');await step(mode+' begin','begin');revision++;
     check(await step(mode+' write','update','周三开会'),'左|周三开会#右');
     await step(mode+' bind','inspect','',{lease:0,id:randomUUID(),action:'bind',epoch:0,version:0,expected:'',selectionStart:0,selectionEnd:0});
     check(await step(mode+' handoff','handoff'),'左|周三开会#右');
     await step(mode+' read','inspect','',command('read'));
     await page.evaluate(mode=>{
      if(mode==='outside-suffix'){window.fixtureEditor.edit(7,8,'外');window.fixtureEditor.select(6);}
      else if(mode==='range-drift'){window.fixtureEditor.edit(0,0,'移');window.fixtureEditor.select(7);}
      else {const old=document.querySelector('#prosemirror');const next=old.cloneNode(true);window.ownedReplacedEditor=old;old.replaceWith(next);(next.matches('[contenteditable]')?next:next.querySelector('[contenteditable]')).focus();}
     },mode);
     const changed=await settled();lastBody=changed.text;
     const row=await step(mode+' acquire','inspect','',command('read'),mode==='outside-suffix');
     if(mode==='outside-suffix'){
      await step(mode+' resume','inspect','',command('acquire'));revision++;
      check(await step(mode+' protected write','update','改好'),'左|改好#外');
      check(await step(mode+' finish','handoff'),'左|改好#外');
     }else{
      if(row.native.reply.applied)throw Error(mode+' unexpectedly preserved capability');
      if(row.settled.text!==changed.text)throw Error(mode+' refusal changed body');
      if(mode==='document-rebuild'&&await page.evaluate(()=>document.querySelector('#prosemirror').textContent)!==changed.text)throw Error('Replacement editor body changed');
     }
     await step(mode+' release','inspect','',command('release'));
     report.boundaryCases.push({mode,passed:true,body:lastBody,code:row.native.reply.code});
     if(mode==='document-rebuild')await page.evaluate(()=>{document.querySelector('#prosemirror').replaceWith(window.ownedReplacedEditor);delete window.ownedReplacedEditor;window.fixtureEditor.select(0);});
     turn++;
    }
   }
   child.stdin.end('exit\n');
   if(!ended)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Native identity probe did not retire')),5000);child.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Native identity cleanup '+code));});});
   if(process.env.SAYWEAVE_BLOCK_BOUNDARY_CASE)return;
   await runActor({page,pid,focus,start,read,settled,trace,report});
   return;
  }
  await step('inspect','inspect');await step('begin','begin');revision++;
  check(await step('first native write','update','周三开会'),'周三开会');
  await step('bind first block','inspect','',{lease:0,id:randomUUID(),action:'bind',epoch:0,version:0,expected:'',selectionStart:0,selectionEnd:0});
  check(await step('first handoff','handoff'),'周三开会');
  await step('read after composition end','inspect','',command('read'));
  for(const [index,replacement,tail,final] of [[0,'周五开会','，不对，改成下周一上午开会','下周一上午开会'],[1,'下周一开会','，改成下午三点开会','下周一下午三点开会'],[2,'下周一下午两点开会','，不对，改成线上四点开会','下周一下午四点线上开会']]){
   const before=await read();
   // A partial user edit, not whole-block deletion or document reconstruction.
   let left=0,right=0;
   while(left<Math.min(before.text.length,replacement.length)&&before.text[left]===replacement[left])left++;
   while(right<Math.min(before.text.length,replacement.length)-left&&before.text.at(-1-right)===replacement.at(-1-right))right++;
   await page.evaluate(({left,end,text,caret})=>{window.fixtureEditor.edit(left,end,text);window.fixtureEditor.select(caret);},{left,end:before.text.length-right,text:replacement.slice(left,replacement.length-right),caret:replacement.length});
   const edited=await settled();lastBody=edited.text;trace.push({label:'user edit '+index,before,settled:edited});
   if(edited.text!==replacement)throw Error('Owned user edit differs');
   await step('read edited block '+index,'inspect','',command('read'));
   const nativeBaseline={...snapshot};
   await step('acquire '+index,'inspect','',command('acquire'));
   for(const [name,body] of [['ASR 1',replacement+tail.slice(0,3)],['ASR 2',replacement+tail],['model',final]]){
    const added=name==='ASR 1'?tail.slice(0,3):tail;
    const requested=baselineMode==='native-readback'&&name!=='model'?nativeBaseline.text.slice(0,nativeBaseline.selectionStart)+added+nativeBaseline.text.slice(nativeBaseline.selectionEnd):body;
    revision++;check(await step(name+' '+index,'update',requested),body);
   }
   check(await step('handoff '+index,'handoff'),final);
  }
  await step('release','inspect','',command('release'));
  report.finalBody=await settled();
  if(report.finalBody.text!==lastBody)throw Error('Late host body change after release');
  child.stdin.end('exit\n');
  if(!ended)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Native probe did not retire')),5000);child.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Native probe cleanup '+code));});});
  await runActor({page,pid,focus,start,read,settled,trace,report});
 }finally{
  report.blockEvents=await page.evaluate(()=>window.ownedBlockEvents);
  writeFileSync(path.join(folder,'block-trace.json'),JSON.stringify({trace,events:report.blockEvents,identity:report.executionIdentity,firstDivergence:report.firstDivergence,errors},null,2));
  if(!ended)child.stdin.end('exit\n');
 }
}

async function runActor({page,pid,focus,start,read:readWhole,settled:settledWhole,trace,report}){
 const executable=process.env.SAYWEAVE_ACTOR_TEST_EXE;
 if(!executable||!path.isAbsolute(executable))throw Error('Built owned actor test executable required');
 const sentinels=process.env.SAYWEAVE_BLOCK_SENTINELS==='1';
 const prefix=sentinels?'左|':'',suffix=sentinels?'#右':'';
 const scoped=sample=>{
  if(!sample.text.startsWith(prefix)||!sample.text.endsWith(suffix))throw Error('Actor changed outside sentinel text');
  return {...sample,fullText:sample.text,text:sample.text.slice(prefix.length,suffix.length?-suffix.length:undefined)};
 };
 const read=async()=>scoped(await readWhole()),settled=async()=>scoped(await settledWhole());
 await page.evaluate(({prefix,suffix})=>window.fixtureEditor.set(prefix+suffix,prefix.length,prefix.length),{prefix,suffix});
 if(process.env.SAYWEAVE_BLOCK_FLOATING_PREDICTION==='1'){await page.evaluate(()=>window.fixturePrediction('示例建议 · 按 Tab 接受'));report.floatingPrediction=await page.locator('[data-owned-prediction]').count()===1;if(!report.floatingPrediction&&!sentinels)throw Error('Floating prediction fixture absent');}
 await settled();
 const actor=start(executable,['--exact','dictation::drafts::tests::owned::richtext_actor','--ignored','--nocapture','--test-threads=1'],{SAYWEAVE_INPUT_ACCEPTANCE:'1',SAYWEAVE_TSF_OWNED_PID:String(pid),SAYWEAVE_DRAFT_STRATEGY:'revisable'});
 let buffer='',diagnostics='',exited=false;const values=[],pending=[];
 actor.stderr.on('data',data=>diagnostics+=data.toString('utf8'));
 actor.stdout.on('data',data=>{buffer+=data.toString('utf8');let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at);buffer=buffer.slice(at+1);const marker=line.indexOf('ACTOR_JSON:');if(marker<0){diagnostics+=line+'\n';continue;}const value=JSON.parse(line.slice(marker+11));if(pending.length)pending.shift().resolve(value);else values.push(value);}});
 actor.once('exit',code=>{exited=true;for(const item of pending.splice(0))item.reject(Error('Actor exited '+code+': '+diagnostics));});
 const receive=()=>values.length?Promise.resolve(values.shift()):new Promise((resolve,reject)=>{if(exited){reject(Error(diagnostics));return;}const item={resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}};const timer=setTimeout(()=>{const index=pending.indexOf(item);if(index>=0)pending.splice(index,1);reject(Error('Actor response timeout: '+diagnostics));},18000);pending.push(item);});
 const command=async value=>{actor.stdin.write(JSON.stringify(value)+'\n');const reply=await receive();trace.push({label:'actor '+value.action,reply});return reply;};
 let lastBody='',cleanupFailure;const segments=[];
 const expectBody=async(label,expected)=>{
  const deadline=Date.now()+5000;let sample;
  do{sample=await settled();trace.push({label,settled:sample});if(sample.text===expected){lastBody=expected;return;}await command({action:'status'});}while(Date.now()<deadline);
  report.firstDivergence={label,kind:'actorBody',expected,actual:sample.text};throw Error(label+': coordinator body differs');
 };
 try{
  report.actorIdentity=await receive();
  const continuity=process.env.SAYWEAVE_BLOCK_CONTINUITY;
  if(continuity){
   if(!['prediction','delete-empty'].includes(continuity)||sentinels)throw Error('Explicit empty-field continuity scenario required');
   report.continuityScenario=continuity;
   for(const [revision,text]of [[1,'周'],[2,'周三'],[3,'周三开会']]){
    segments[0]={id:1,revision,text,confirmed:false};
    await command({action:'asr',segments});await expectBody('continuity stream '+revision,text);
   }
   if(continuity==='delete-empty'){
    await command({action:'edited'});
    const repeated=process.env.SAYWEAVE_BLOCK_DELETE_KEYS==='repeat';
    const keys=repeated?Array.from({length:4},()=>['Backspace',true]).concat([['Backspace',false]]):[['ControlLeft',true],['KeyA',true],['KeyA',false],['ControlLeft',false],['Backspace',true],['Backspace',false]];
    report.continuityDeletion=repeated?'held-repeated-backspace':'selected-backspace';
    let remaining='周三开会';
    for(const [key,down]of keys){
     const result=await focus({action:'fixture-key',process:pid,key,down});
     if(!result.keyResult?.accepted)throw Error('Guarded empty-field deletion failed');
     if(repeated&&down){remaining=remaining.slice(0,-1);await expectBody('continuity held deletion '+remaining.length,remaining);}
    }
    await expectBody('continuity actual deletion','');
    segments[0].confirmed=true;
    segments.push({id:2,revision:1,text:'新的安排',confirmed:true});
    await command({action:'asr',segments});await expectBody('continuity after delete','新的安排。');
   }else{
    await command({action:'edited'});
    for(const key of ['ArrowLeft','ArrowLeft','Backspace'])for(const down of [true,false]){
     const result=await focus({action:'fixture-key',process:pid,key,down});
     if(!result.keyResult?.accepted)throw Error('Guarded new-block editing failed');
    }
    await expectBody('continuity new-block deletion','周开会');
    const typed=await focus({action:'fixture-text',process:pid,text:'五'});
    if(!typed.keyResult?.accepted)throw Error('Guarded new-block text edit failed');
    await expectBody('continuity new-block edit','周五开会');
    segments[0].confirmed=true;
    segments.push({id:2,revision:1,text:'地点在上海',confirmed:true});
    await command({action:'asr',segments});await expectBody('continuity new-block polish','周五开会，地点在上海。');
   }
   report.actorDelivery=await command({action:'finish'});
   const final=continuity==='prediction'?'周五开会，地点在上海。':'新的安排。';
   await expectBody('continuity final settled',final);
   if(report.actorDelivery.error||report.actorDelivery.resultOnly||report.actorDelivery.delivery?.restoreError)throw Error('Continuity finished without safe direct input');
   actor.stdin.end();
   if(!exited)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Continuity cleanup timeout')),5000);actor.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Continuity actor failed: '+diagnostics));});});
   report.actorPassed=true;
   return;
  }
  const earlyEdit=process.env.SAYWEAVE_BLOCK_EARLY_EDIT==='1';
  const firstOnly=process.env.SAYWEAVE_BLOCK_FIRST_ONLY==='1';
  const commitEarly=process.env.SAYWEAVE_BLOCK_COMMIT_EARLY==='1';
  if(commitEarly&&(!process.env.SAYWEAVE_INITIAL_BARRIER||earlyEdit||process.env.SAYWEAVE_BLOCK_BACKSPACE!=='1'))throw Error('Initial commit barrier requires direct Backspace and separate receipt timing');
  report.actorScenario=earlyEdit?'edit-before-tracking-receipt':'edit-after-tracking-receipt';
  const backspace=process.env.SAYWEAVE_BLOCK_BACKSPACE==='1';
  const interiorEdit=process.env.SAYWEAVE_BLOCK_INTERIOR_EDIT==='1';
  if(interiorEdit&&(backspace||earlyEdit||commitEarly))throw Error('Interior-edit round trip requires the ordinary bound block');
  report.actorInteriorEditRoundTrip=interiorEdit;
  const userNewline=process.env.SAYWEAVE_BLOCK_USER_NEWLINE==='1';
  if(userNewline&&!backspace)throw Error('User newline regression requires direct Backspace setup');
  if(userNewline)report.actorNewlineRepresentation='paragraph';
  const retained=userNewline?'\n':'';
  const firstText=backspace?'周三开会。':'周三开会';
  segments.push({id:1,revision:1,text:firstText,confirmed:false});
  if(process.env.SAYWEAVE_BLOCK_INITIAL_STREAM==='1'){
   if(earlyEdit)throw Error('Initial streaming must not bypass the first-write edit barrier');
   for(const text of ['周','周三','周三开']){
    segments[0].text=text;segments[0].revision++;
    await command({action:'asr',segments});await expectBody('actor initial stream '+text,text);
   }
   segments[0].text=firstText;segments[0].revision++;
  }
  await command({action:'asr',segments});await expectBody('actor first input',firstText);
  if(earlyEdit){
   const deadline=Date.now()+5000;
   while(!diagnostics.includes('owned_bind_barrier=waiting')){
    if(Date.now()>=deadline)throw Error('Early-edit receipt barrier not reached');
    await command({action:'status'});await settled();
   }
  }
  // Update now installs native tracking before returning its receipt. Keep
  // both timings: edit before the client consumes that receipt, or after it.
  const bindDeadline=Date.now()+5000;
  while(!earlyEdit&&!commitEarly&&!diagnostics.includes('operation=draft_block_bound')){
   if(Date.now()>=bindDeadline)throw Error('Production initial block did not bind');
   await command({action:'status'});await settled();
  }
  report.actorTrackingReceiptBeforeEdit=diagnostics.includes('operation=draft_block_bound');
  if(commitEarly){report.firstWriteBeforeNativeReceipt=!diagnostics.includes('revision=1 action=Update applied=true');if(!report.firstWriteBeforeNativeReceipt)throw Error('Initial commit barrier did not hold the native operation receipt');}
  if(earlyEdit&&report.actorTrackingReceiptBeforeEdit)throw Error('Early-edit timing not exercised: tracking receipt already consumed');
  for(const [index,replacement,tail,final]of (firstOnly?[]:[[0,backspace?'周三开会':'周五开会','，不对，改成下周一上午开会','下周一上午开会'],[1,'下周一开会','，改成下午三点开会','下周一下午三点开会'],[2,'下周一下午两点开会','，不对，改成线上四点开会','下周一下午四点线上开会']])){
   const before=await read();if(before.text!==lastBody)throw Error('Late actor body change before edit');
   const expectedEdit=replacement+(index===0?'':retained);
   let left=0,right=0;while(left<Math.min(before.text.length,expectedEdit.length)&&before.text[left]===expectedEdit[left])left++;while(right<Math.min(before.text.length,expectedEdit.length)-left&&before.text.at(-1-right)===expectedEdit.at(-1-right))right++;
   const eventWatermark=await page.evaluate(()=>window.ownedBlockEvents.length);
   const domWasComposing=await page.evaluate(()=>window.fixtureEditor.inspect().composing);
   if(!(backspace&&index===0))await command({action:'edited'});
   // End the native composition through the owned host's actual key route.
   // Mutating ProseMirror while TSF still owns a stale composition bypasses
   // normal editing and can itself manufacture an old-text commit on handoff.
   for(const down of [true,false])await focus({action:'fixture-key',process:pid,key:backspace&&index===0?'Backspace':'ArrowLeft',down});
   if(backspace&&index===0)await command({action:'edited'});
   if(commitEarly&&index===0)writeFileSync(process.env.SAYWEAVE_INITIAL_BARRIER,'continue');
   if(domWasComposing)await page.waitForFunction(at=>window.ownedBlockEvents.slice(at).some(e=>e.type==='compositionend'),eventWatermark,{timeout:5000});
   const handedOff=await settled();trace.push({label:'actor edit handoff '+index,settled:handedOff});
   if(handedOff.text!==(backspace&&index===0?replacement:before.text))throw Error('Native editing handoff changed body');
   if(process.env.SAYWEAVE_BLOCK_SELECTION_PAUSE==='1'&&!(backspace&&index===0)){
    // Real editing starts by moving the caret/selection, before changing text.
    // Keep this interval long enough for the production reader to see it.
    await page.evaluate(at=>window.fixtureEditor.select(at),prefix.length+left);
    const logStart=diagnostics.length,deadline=Date.now()+5000;
    while(!diagnostics.slice(logStart).includes('operation=draft_selection_wait')){
     if(diagnostics.slice(logStart).includes('code=blockReleased')){
      report.firstDivergence={label:'selection before edit '+index,kind:'prematureBlockRelease',actual:(await read()).text};
      throw Error('Caret-only positioning retired the original block before manual edit');
     }
     if(Date.now()>=deadline)throw Error('Caret-only read was not observed');
     await command({action:'status'});await settled();
    }
    const positioned=await settled();trace.push({label:'actor selection before edit '+index,settled:positioned});
    if(positioned.text!==before.text)throw Error('Caret-only positioning changed body');
   }
   if(!(backspace&&index===0))await page.evaluate(({left,end,text,caret})=>{window.fixtureEditor.edit(left,end,text);window.fixtureEditor.select(caret);},{left:prefix.length+left,end:prefix.length+before.text.length-right,text:expectedEdit.slice(left,expectedEdit.length-right),caret:prefix.length+(interiorEdit&&index===0?expectedEdit.length-right:replacement.length)});
   // Use this editor's supported paragraph break. A raw LF inserted into a
   // text node is normalized to a space by its own DOM parser, even during
   // ordinary typing without TSF. The separate host-control evidence retains
   // that failure; it is not a user paragraph-break acceptance case.
   if(userNewline&&index===0)await page.evaluate(at=>{window.fixtureEditor.appendParagraph();window.fixtureEditor.select(at);},prefix.length+replacement.length);
   const edited=await settled();trace.push({label:'actor manual edit '+index,settled:edited});if(edited.text!==replacement+retained)throw Error('Actor manual edit differs');
   if(index===0){
    report.actorTrackingReceiptAfterEdit=diagnostics.includes('operation=draft_block_bound');
    if(earlyEdit&&report.actorTrackingReceiptAfterEdit)throw Error('Early-edit timing ambiguous: tracking receipt consumed during edit');
    if(earlyEdit)await command({action:'release-bind'});
   }
   segments.forEach(s=>s.confirmed=true);
   segments.push({id:index+2,revision:1,text:tail.slice(0,3),confirmed:false});
   // The first edit stays inside the old body through reacquisition. Its next
   // keyboard handoff must preserve the complete model rewrite, including the
   // suffix after that old caret. Subsequent rounds retain partial streaming.
   if(!interiorEdit||index!==0){await command({action:'asr',segments});await expectBody('actor partial '+index,replacement+tail.slice(0,3)+retained);}
   segments.at(-1).revision++;segments.at(-1).text=tail;segments.at(-1).confirmed=true;
   await command({action:'asr',segments});await expectBody('actor model '+index,final+retained);
   if(process.env.SAYWEAVE_BLOCK_PRESENTATION==='1'){
    const state=await command({action:'status'});
    if(state.published!=='recording'||state.audioRoute!==1)throw Error('Body resumed without published recording and retained audio route: '+JSON.stringify(state));
   }
  }
  if(process.env.SAYWEAVE_BLOCK_FINISH_SELECTION==='1'){
   const eventWatermark=await page.evaluate(()=>window.ownedBlockEvents.length);
   await command({action:'edited'});
   for(const down of [true,false])await focus({action:'fixture-key',process:pid,key:'ArrowLeft',down});
   await page.waitForFunction(at=>window.ownedBlockEvents.slice(at).some(e=>e.type==='compositionend'),eventWatermark,{timeout:5000});
   await page.evaluate(at=>window.fixtureEditor.select(at),prefix.length+1);
   const logStart=diagnostics.length,deadline=Date.now()+5000;
   while(!diagnostics.slice(logStart).includes('operation=draft_selection_wait')){
    if(diagnostics.slice(logStart).includes('code=blockReleased'))throw Error('Final caret move discarded the block');
    if(Date.now()>=deadline)throw Error('Final caret-only read was not observed');
    await command({action:'status'});await settled();
   }
  }
  const finishStarted=Date.now();
  report.actorDelivery=await command({action:'finish'});
  report.actorFinishMs=Date.now()-finishStarted;
  if(process.env.SAYWEAVE_BLOCK_FINISH_SELECTION==='1'&&report.actorFinishMs>3000)throw Error('Caret-only finish waited for another edit');
  if(!report.actorDelivery.finished||report.actorDelivery.error||report.actorDelivery.requests!==(firstOnly?1:3))throw Error('Actor final settlement or request count differs');
  await expectBody('actor final settled',firstOnly?'下周一上午开会':lastBody);
  actor.stdin.end();
  if(!exited)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Actor cleanup timeout')),5000);actor.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Actor failed '+code+': '+diagnostics));});});
  report.actorPassed=true;
 }finally{
  const samplingErrors=[];
  try{report.actorBodyBeforeCleanup=await settled();}catch(error){samplingErrors.push(String(error));}
  try{
   if(!exited){actor.stdin.end();await new Promise(resolve=>{const timer=setTimeout(resolve,5500);actor.once('exit',()=>{clearTimeout(timer);resolve();});});}
  }finally{
   report.actorDiagnostics=diagnostics;
  }
  try{report.actorBodyAfterCleanup=await settled();}catch(error){samplingErrors.push(String(error));}
  if(samplingErrors.length){report.actorCleanupSamplingErrors=samplingErrors;cleanupFailure=Error('Actor cleanup body evidence unavailable');}
 }
 if(cleanupFailure)throw cleanupFailure;
}
