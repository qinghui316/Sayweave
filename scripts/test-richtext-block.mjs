// Scenario extension of test-prefix-input; all text belongs to its owned fixture.
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {writeFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';

// Diagnostic control for the exact held-key burst used by raw E. No dictation
// actor or native write is started; keep this separate from input acceptance.
export async function runDeletionBurstControl({page,pid,focus,report}){
 const held=new Set(),actions=[];
 report.deletionBurstControl={dictation:false,actions};
 await page.evaluate(()=>{
  window.fixtureEditor.set('周三开会',4,4);window.ownedBurstEvents=[];
  for(const type of ['keydown','keyup','beforeinput','input'])document.addEventListener(type,event=>{
   if(event.target.closest?.('#prosemirror'))queueMicrotask(()=>window.ownedBurstEvents.push({type,key:event.key,inputType:event.inputType,body:window.fixtureEditor.read(),at:performance.now()}));
  },true);
 });
 await page.bringToFront();
 if((await focus({action:'focus-process',process:pid})).foregroundProcess!==pid)throw Error('Deletion control lost its owned foreground');
 const key=async down=>{
  if(down)held.add('Backspace');
  const result=await focus({action:'fixture-key',process:pid,key:'Backspace',down});
  if(!result.keyResult?.accepted)throw Error('Deletion control native key rejected');
  if(!down)held.delete('Backspace');
  actions.push({down,at:Date.now()});
 };
 try{for(let count=0;count<4;count++)await key(true);}finally{if(held.size)await key(false);}
 const sample=await page.evaluate(async()=>{
  await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
  return {body:window.fixtureEditor.read(),events:window.ownedBurstEvents,selection:window.fixtureEditor.inspect().semanticSelection};
 });
 Object.assign(report.deletionBurstControl,sample,{pendingKeys:[...held]});
 if(sample.body!=='')throw Error('Four accepted native held-key packets did not delete four characters without dictation');
 report.deletionBurstControl.passed=true;
}

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
  for(const type of ['keydown','keyup','compositionstart','compositionupdate','compositionend','beforeinput','input'])document.addEventListener(type,event=>{
   if(!event.target.closest?.('#prosemirror'))return;
   const targets=Array.from(event.getTargetRanges?.()??[]).slice(0,16).map(range=>({start:range.startOffset,end:range.endOffset,startParent:range.startContainer.parentElement?.tagName,endParent:range.endContainer.parentElement?.tagName}));
   const record=phase=>window.ownedBlockEvents.push({type,phase,at:performance.now(),trusted:event.isTrusted,key:event.key,keyCode:event.keyCode,targets,data:event.data,inputType:event.inputType,body:window.fixtureEditor.read(),dom:window.fixtureEditor.inspect()});
   record('capture');queueMicrotask(()=>record('microtask'));setTimeout(()=>record('after-task'),0);
  },true);
 });
 const child=process.env.SAYWEAVE_BLOCK_RAW_ENTRY?null:start(path.join(root,'src-tauri/text-service/target/debug/tsf-probe.exe'),['richtext-block-script',String(pid)],{SAYWEAVE_INPUT_ACCEPTANCE:'1',SAYWEAVE_TSF_OWNED_PID:String(pid)});
 const queue=[],pending=[];let buffer='',errors='',ended=false,probeExit;
 child?.stdout.on('data',data=>{buffer+=data.toString('utf8');let i;while((i=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,i);buffer=buffer.slice(i+1);if(!line.trim())continue;try{const value=JSON.parse(line);if(pending.length)pending.shift().resolve(value);else queue.push(value);}catch(error){for(const item of pending.splice(0))item.reject(error);}}});
 child?.stderr.on('data',data=>{errors+=data.toString('utf8');});
 child?.once('exit',code=>{ended=true;probeExit=code;for(const item of pending.splice(0))item.reject(Error('Owned block probe exited '+code+': '+errors));});
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
 if(process.env.SAYWEAVE_BLOCK_RAW_ENTRY){
  report.nativePrewarm=false;
  try{await runActorSequence({page,pid,focus,start,read,settled,trace,report,folder});}
  finally{writeFileSync(path.join(folder,'block-trace.json'),JSON.stringify({trace,events:await page.evaluate(()=>window.ownedBlockEvents).catch(()=>[])},null,2));}
  return;
 }
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
 const sourceCase=process.env.SAYWEAVE_BLOCK_SOURCE_CASE;
 let sourceExpected,sourceOther,sourceOriginalText,sourceReplacementText,sourceFailure,sourceCleanupFailure;
 await (async()=>{try{
  report.executionIdentity=await next();
  if(sourceCase){
   const ordinarySource=process.env.SAYWEAVE_BLOCK_SOURCE_ORDINARY==='1';
   if(!['clear-retype','paragraph-rebuild','document-rebuild','other-field','boundary-insert','outside-tail','external-same-text'].includes(sourceCase))throw Error('Unknown source-boundary case');
   await page.evaluate(mode=>{
    if(mode==='external-same-text')window.fixtureEditor.set('周三开会',4,4);
    else {window.fixtureEditor.set('',0,0);window.fixturePrediction('示例建议 · 按 Tab 接受');}
   },sourceCase);
   lastBody=(await settled()).text;revision=0;
   await step('source inspect','inspect');await step('source begin','begin');revision++;
   const bind={lease:0,id:randomUUID(),action:'bind',epoch:0,version:0,expected:'',selectionStart:0,selectionEnd:0};
   check(await step('source first write','update',ordinarySource?'周三开会':'周',bind),(sourceCase==='external-same-text'?'周三开会':'')+(ordinarySource?'周三开会':'周'));
   if(snapshot?.id!==bind.id||!(snapshot.epoch>0))throw Error('Source fixture block identity was not bound');
   if(!ordinarySource){revision++;
    check(await step('source second write','update','周三开会'),sourceCase==='external-same-text'?'周三开会周三开会':'周三开会');
   }
   for(const down of [true,false]){const key=await focus({action:'fixture-key',process:pid,key:'ArrowLeft',down});if(!key.keyResult?.accepted)throw Error('Source handoff key refused');}
   await settled();const handedOff=await step('source handed off','handoff');
   const inspection=await step('source readonly stage evidence','inspect');
   const originText=inspection.native.reply.code.split(' paragraphOriginState=')[1];
   const origin=originText?JSON.parse(originText):null;
   const installed=ordinarySource?/sourceReceipt=committed(?:\.handoff)?\//.test(handedOff.native.reply.code)&&handedOff.native.reply.code.includes('nativeReceipt.paired'):(origin?.savedRangeComparisons?.some(sample=>sample.handoffStage==='tracking.committed'&&sample.facts?.trackingCommitted===true)??false);
   report.sourceBoundary={case:sourceCase,sourceStage:origin,ordinarySource,preparation:'explicit source boundary fixture; not an original-operation timing test',handoffCode:handedOff.native.reply.code,readonlyInstalled:installed,block:{id:snapshot.id,epoch:snapshot.epoch}};
   if(sourceCase!=='external-same-text'&&(!installed||!ordinarySource&&(!origin.tracking?.valid||!origin.tracking?.installed||!origin.tracking?.selectionPending||origin.tracking?.writable||origin.tracking?.id!==snapshot.id||origin.tracking?.epoch!==snapshot.epoch)))throw Error('Current readonly source premise not established; no deep boundary coverage');
   if(ordinarySource&&sourceCase!=='external-same-text'){
    const current=await step('ordinary committed source read','inspect','',command('read'));
    if(current.native.reply.block?.id!==bind.id||current.native.reply.block?.text!=='周三开会'||current.native.reply.block?.writable!==false)throw Error('Ordinary source read did not retain the original readonly block');
   }
   const originalSourceAcquire=command('acquire');
   await page.evaluate(()=>{const root=document.querySelector('#prosemirror');window.ownedSourceEditor=root.matches('[contenteditable="true"]')?root:root.querySelector('[contenteditable="true"]');});
   if(sourceCase==='document-rebuild')sourceOriginalText=await page.evaluate(()=>window.ownedSourceEditor.textContent);
   if(sourceCase==='external-same-text'){
    await page.evaluate(()=>window.fixtureEditor.select(8));lastBody=(await settled()).text;
    await step('external prefix read','inspect','',command('read'));await step('external prefix acquire','inspect','',command('acquire'));revision++;
    check(await step('external prefix protected update','update','下周一上午开会'),'周三开会下周一上午开会');
    sourceExpected='周三开会下周一上午开会';
    check(await step('external prefix final handoff','handoff'),sourceExpected);
   }else if(sourceCase==='paragraph-rebuild'){
    // Changing a node view preserves this EditorState/document and is not a
    // document-identity loss. Test that continuity separately from replacement.
    sourceExpected='周三开会';
    const rebuilt=await page.evaluate(()=>{
     const root=document.querySelector('#prosemirror'),paragraph=root.querySelector('p');
     const documentBefore=JSON.stringify(window.fixtureEditor.inspect().document);
     window.fixtureEditor.rebuildParagraph();
     return {sameEditor:root===window.ownedSourceEditor.parentElement||root===window.ownedSourceEditor,paragraphReplaced:paragraph!==root.querySelector('p'),sameDocument:documentBefore===JSON.stringify(window.fixtureEditor.inspect().document)};
    });
    if(!rebuilt.sameEditor||!rebuilt.paragraphReplaced||!rebuilt.sameDocument)throw Error('View-only paragraph rebuild premise missing');
    lastBody=(await settled()).text;
    const current=await step('view-only rebuild continuous read','inspect','',command('read'));
    check(current,sourceExpected);
    if(current.native.reply.block?.id!==bind.id||current.native.reply.block?.epoch!==originalSourceAcquire.epoch||current.native.reply.block?.version<originalSourceAcquire.version||current.native.reply.block?.writable!==false||current.native.reply.block?.text!=='周三开会')throw Error('View-only rebuild lost its committed readonly range');
    Object.assign(report.sourceBoundary,{viewOnly:rebuilt,continuous:true,body:lastBody});
   }else if(sourceCase==='boundary-insert'){
    // At the exact start, backward gravity can include a legitimate user
    // insertion. This is preservation coverage, not a block-outside refusal.
    await page.evaluate(()=>window.fixtureEditor.edit(0,0,'外|'));
    lastBody=(await settled()).text;
    if(lastBody!=='外|周三开会')throw Error('Boundary insertion body differs');
    await step('boundary insertion read','inspect','',command('read'));
    await step('boundary insertion acquire','inspect','',command('acquire'));revision++;
    sourceExpected='外|下周一上午开会';
    check(await step('boundary insertion rewrite','update',sourceExpected),sourceExpected);
    check(await step('boundary insertion final handoff','handoff'),sourceExpected);
   }else{
    // No native query occurs between these mutations. The ordinary mode's
    // explicit prepared Read above is not raw or pending-publication coverage.
    await page.evaluate(mode=>{
     if(mode==='clear-retype'){window.fixtureEditor.edit(0,4,'');}
     else if(mode==='document-rebuild'){
      const original=document.querySelector('#prosemirror'),replacement=original.cloneNode(true);
      window.ownedSourceDocument=original;window.ownedSourceReplacement=replacement;original.replaceWith(replacement);
      const editor=replacement.matches('[contenteditable="true"]')?replacement:replacement.querySelector('[contenteditable="true"]');editor.focus();
      const selection=window.getSelection(),range=document.createRange();range.selectNodeContents(editor);range.collapse(false);selection.removeAllRanges();selection.addRange(range);
      if(original.isConnected||!editor.isConnected||editor===window.ownedSourceEditor||document.activeElement!==editor||!editor.contains(selection.anchorNode)||!editor.contains(selection.focusNode))throw Error('Document replacement premise missing');
     }
     else if(mode==='other-field'){document.querySelector('#input').value='周三开会';document.querySelector('#input').focus();}
     else {window.fixtureEditor.appendParagraph();window.fixtureEditor.select(window.fixtureEditor.read().length);}
    },sourceCase);
    await settled();
    if(sourceCase==='clear-retype'){await page.evaluate(()=>window.fixtureEditor.edit(0,0,'周三开会'));await settled();}
    const expectedAfterMutation=(await settled()).text;
    sourceExpected=expectedAfterMutation;
    if(sourceCase==='other-field')sourceOther=await page.locator('#input').inputValue();
    if(sourceCase==='document-rebuild')sourceReplacementText=await page.evaluate(()=>window.ownedSourceReplacement.textContent);
    lastBody=expectedAfterMutation;
    const refused=await step('source boundary rejects old block','inspect','',command('read'),false);
    const code=refused.native.reply.code;
    const permanent=reply=>{
     if(reply.applied||reply.mutation!=='notAttempted')return false;
     if(/^blockUnavailable(?:[: ]|$)/.test(reply.code))return true;
     try{
      const encoded=reply.code.split(':structure=')[1];
      const metadata=encoded.indexOf(' observationReads=');
      const first=JSON.parse(metadata<0?encoded:encoded.slice(0,metadata)).first;
      return first?.end_valid===false&&first.error===0x80040504;
     }catch{return false;}
    };
    const continuous=sourceCase==='clear-retype'&&refused.native.reply.applied;
    if(continuous){
     const current=refused.native.reply.block;
     if(current?.id!==bind.id||current.epoch!==originalSourceAcquire.epoch||current.version<=originalSourceAcquire.version||current.writable!==false||current.text!==expectedAfterMutation)throw Error('Retyped source lacks continuous original-range/version evidence');
    }else if(refused.native.reply.applied||refused.native.reply.mutation!=='notAttempted')throw Error('Invalid source was accepted or mutated: '+code);
    if(refused.settled.text!==expectedAfterMutation)throw Error('Source check changed the body');
    if(sourceOther!==undefined&&await page.locator('#input').inputValue()!==sourceOther)throw Error('Replacement field changed after old source refusal');
    if(sourceReplacementText!==undefined&&await page.evaluate(()=>window.ownedSourceReplacement.textContent)!==sourceReplacementText)throw Error('Replacement document changed after old source refusal');
    if(sourceOriginalText!==undefined&&await page.evaluate(()=>window.ownedSourceEditor.textContent)!==sourceOriginalText)throw Error('Original document DOM changed after old source refusal');
    const restored=await page.evaluate(({mode,start,end})=>{
     if(mode==='document-rebuild'){
      const current=document.querySelector('#prosemirror');if(!current||!window.ownedSourceDocument)throw Error('Original document restore premise missing');
      current.replaceWith(window.ownedSourceDocument);delete window.ownedSourceDocument;
     }
     const root=document.querySelector('#prosemirror');const editor=root.matches('[contenteditable="true"]')?root:root.querySelector('[contenteditable="true"]');
     editor.focus();window.fixtureEditor.select(start,end);
     const selection=window.getSelection();
     // The model selection can already equal the frozen value even when
     // focus restoration put the browser caret elsewhere. Set and verify the
     // actual DOM selection; this prepared check is not a raw input stimulus.
     const point=at=>{const walker=document.createTreeWalker(editor,NodeFilter.SHOW_TEXT);let node,left=at;while((node=walker.nextNode())){if(left<=node.textContent.length)return [node,left];left-=node.textContent.length;}throw Error('Original source selection endpoint missing');};
     const [anchor,anchorAt]=point(start),[focus,focusAt]=point(end),restoredRange=document.createRange();
     restoredRange.setStart(anchor,anchorAt);restoredRange.setEnd(focus,focusAt);selection.removeAllRanges();selection.addRange(restoredRange);
     const offset=(node,at)=>{const range=document.createRange();range.selectNodeContents(editor);range.setEnd(node,at);return range.toString().length;};
     return {sameEditor:editor===window.ownedSourceEditor,focused:document.activeElement===editor,selectionWithin:editor.contains(selection?.anchorNode)&&editor.contains(selection?.focusNode),selected:[offset(selection.anchorNode,selection.anchorOffset),offset(selection.focusNode,selection.focusOffset)]};
    },{mode:sourceCase,start:originalSourceAcquire.selectionStart,end:originalSourceAcquire.selectionEnd});
    report.sourceBoundary.restoration=restored;
    if(!restored.sameEditor||!restored.focused||!restored.selectionWithin||restored.selected[0]!==originalSourceAcquire.selectionStart||restored.selected[1]!==originalSourceAcquire.selectionEnd)throw Error('Original source editor/focus/exact snapshot selection was not restored');
    lastBody=(await settled()).text;
    if(lastBody!==expectedAfterMutation)throw Error('Source focus restoration changed text');
    const late=await step('original source late acquire after restoration','inspect','',originalSourceAcquire,false);
    if(continuous){
     if(late.native.reply.applied||late.native.reply.mutation!=='notAttempted')throw Error('Stale source version regained write permission');
    }else if(!permanent(late.native.reply))throw Error('Original source was not permanently retired after restoring original focus: '+late.native.reply.code);
    if(late.settled.text!==expectedAfterMutation||sourceOther!==undefined&&await page.locator('#input').inputValue()!==sourceOther)throw Error('Late old source operation changed a field');
    if(sourceReplacementText!==undefined&&await page.evaluate(()=>window.ownedSourceReplacement.textContent)!==sourceReplacementText)throw Error('Late old source operation changed replacement document');
    if(sourceOriginalText!==undefined&&await page.evaluate(()=>window.ownedSourceEditor.textContent)!==sourceOriginalText)throw Error('Late old source operation changed original document DOM');
    Object.assign(report.sourceBoundary,{code,lateCode:late.native.reply.code,continuous,body:expectedAfterMutation});
   }
   await step('source release','inspect','',command('release'));
   child.stdin.end('exit\n');
   if(!ended)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Source boundary cleanup deadline')),5000);child.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Source boundary cleanup '+code));});});
   if(probeExit!==0)throw Error('Source boundary probe exit '+probeExit);
   if((await settled()).text!==sourceExpected)throw Error('Source cleanup changed the frozen body');
   report.actorPassed=true;
   return;
  }
  if(process.env.SAYWEAVE_BLOCK_ACTOR_ONLY==='1'){
   if(process.env.SAYWEAVE_BLOCK_BOUNDARIES==='1'){
    report.boundaryCases=[];
    const cases=['outside-suffix','range-drift','document-rebuild','other-field','other-field-same-text'];
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
      else if(mode.startsWith('other-field')){const input=document.querySelector('#input');input.value=mode==='other-field-same-text'?'左|周三开会#右':'另一输入框';input.focus();}
      else {const old=document.querySelector('#prosemirror');const next=old.cloneNode(true);window.ownedReplacedEditor=old;old.replaceWith(next);(next.matches('[contenteditable]')?next:next.querySelector('[contenteditable]')).focus();}
     },mode);
     const changed=await settled();lastBody=changed.text;
     const other=mode.startsWith('other-field')?await page.locator('#input').inputValue():undefined;
     const row=await step(mode+' acquire','inspect','',command('read'),mode==='outside-suffix');
     if(mode==='outside-suffix'){
      await step(mode+' resume','inspect','',command('acquire'));revision++;
      check(await step(mode+' protected write','update','改好'),'左|改好#外');
      check(await step(mode+' finish','handoff'),'左|改好#外');
     }else{
      const permanentlyRefused=reply=>{
       if(reply.applied||reply.mutation!=='notAttempted')return false;
       if(/^block(?:Unavailable|ScopeInvalid|DocumentFocus|SnapshotField|EditRangeDrift)/.test(reply.code))return true;
       // The first edit callback can expose its exact boundary failure before
       // the next request reports blockUnavailable. Require actual invalidated
       // native state; a representation wait is not permanent rejection.
       if(!/^block(?:Prefix|Suffix)Text:/.test(reply.code))return false;
       try{
        const encoded=reply.code.split(':structure=')[1];
        // The fixture appends readonly diagnostics after the native JSON.
        const metadata=encoded.indexOf(' observationReads=');
        const state=JSON.parse(metadata<0?encoded:encoded.slice(0,metadata)).first;
        return state?.end_valid===false&&state.error===0x80040504;
       }catch{return false;}
      };
      if(!permanentlyRefused(row.native.reply))throw Error(mode+' did not permanently revoke the old capability');
      if(row.settled.text!==changed.text)throw Error(mode+' refusal changed body');
      if(mode==='document-rebuild'&&await page.evaluate(()=>document.querySelector('#prosemirror').textContent)!==changed.text)throw Error('Replacement editor body changed');
      if(other!==undefined&&await page.locator('#input').inputValue()!==other)throw Error(mode+' first refusal changed the other field');
      // Restore the original object/focus after the permanent refusal, then
      // retry the old capability. Same text/focus cannot revive its lease.
      const restored=await page.evaluate(mode=>{
       if(mode==='document-rebuild'){document.querySelector('#prosemirror').replaceWith(window.ownedReplacedEditor);delete window.ownedReplacedEditor;}
       const host=document.querySelector('#prosemirror');
       const editor=host.matches('[contenteditable="true"]')?host:host.querySelector('[contenteditable="true"]');
       editor.focus();
       window.fixtureEditor.select(mode==='range-drift'?7:6);
       const selection=window.getSelection();
       return {focused:document.activeElement===editor,selectionWithin:editor.contains(selection?.anchorNode)&&editor.contains(selection?.focusNode)};
      },mode);
      if(!restored.focused||!restored.selectionWithin)throw Error(mode+' original field focus was not restored');
      const stale=await step(mode+' late acquire','inspect','',command('acquire'),false);
      if(!permanentlyRefused(stale.native.reply)||stale.settled.text!==changed.text)throw Error(mode+' revived an invalid old capability');
      if(other!==undefined&&await page.locator('#input').inputValue()!==other)throw Error(mode+' changed the other field');
     }
     await step(mode+' release','inspect','',command('release'));
     report.boundaryCases.push({mode,passed:true,body:lastBody,code:row.native.reply.code});
     turn++;
    }
   }
   child.stdin.end('exit\n');
   if(!ended)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Native identity probe did not retire')),5000);child.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Native identity cleanup '+code));});});
   if(process.env.SAYWEAVE_BLOCK_BOUNDARY_CASE)return;
   await runActorSequence({page,pid,focus,start,read,settled,trace,report,folder});
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
  await runActorSequence({page,pid,focus,start,read,settled,trace,report,folder});
 }catch(error){sourceFailure=error;throw error;}finally{
  report.blockEvents=await page.evaluate(()=>window.ownedBlockEvents);
  writeFileSync(path.join(folder,'block-trace.json'),JSON.stringify({trace,events:report.blockEvents,identity:report.executionIdentity,firstDivergence:report.firstDivergence,errors},null,2));
  if(!ended)child.stdin.end('exit\n');
  if(sourceCase){
   const sourceCleanupErrors=[];
   if(!ended)await new Promise(resolve=>{const timer=setTimeout(resolve,5000);child.once('exit',()=>{clearTimeout(timer);resolve();});});
   report.sourceProbeExit=probeExit;
   try{if(sourceExpected!==undefined){
    report.sourceBodyAfterCleanup=(await settled()).text;
    if(report.sourceBodyAfterCleanup!==sourceExpected||sourceOther!==undefined&&await page.locator('#input').inputValue()!==sourceOther||sourceReplacementText!==undefined&&await page.evaluate(()=>window.ownedSourceReplacement.textContent)!==sourceReplacementText)sourceCleanupErrors.push('Source refusal/retirement changed original or replacement field');
    if(sourceOriginalText!==undefined&&await page.evaluate(()=>window.ownedSourceEditor.textContent)!==sourceOriginalText)sourceCleanupErrors.push('Source retirement changed original document DOM');
   }}catch(error){sourceCleanupErrors.push(String(error));}
   if(probeExit!==0)sourceCleanupErrors.push('Source probe did not retire successfully: '+probeExit);
   report.sourceCleanupErrors=sourceCleanupErrors;
   if(sourceCleanupErrors.length)sourceCleanupFailure=Error(sourceCleanupErrors.join('; '));
  }
 }})();
 if(sourceCleanupFailure&&!sourceFailure)throw sourceCleanupFailure;
}

async function runActorSequence(args){
 const corrected=await runActor(args);
 if(!process.env.SAYWEAVE_ACTOR_LEARNING_RESTART)return;
 const {settled,trace,report}=args;
 if(typeof corrected!=='string')throw Error('Correction restart did not finish its first task');
 report.correctionFirstTask={identity:report.actorIdentity,diagnostics:report.actorDiagnostics,bodyAfterCleanup:report.actorBodyAfterCleanup};
 const retained=await settled();
 trace.push({label:'corrected word after retiring the prior task',settled:retained});
 if(retained.text!==corrected)throw Error('Prior task retirement changed the corrected word');
 await runActor({...args,restartBody:corrected});
 if(!report.correctionRestartPassed)throw Error('Correction restart did not complete the second task');
 if(report.actorIdentity.task===report.correctionFirstTask.identity.task||report.actorIdentity.process!==report.correctionFirstTask.identity.process)throw Error('Correction restart task/editor identities differ from the required sequence');
 if(process.env.SAYWEAVE_ACTOR_ADJACENT_IME==='1'||process.env.SAYWEAVE_ACTOR_LEARNING_RESTART==='shifted'){
  const before=await settled();
  const tap=async key=>{for(const down of [true,false]){const reply=await args.focus({action:'fixture-key',process:args.pid,key,down});if(!reply.keyResult?.accepted)throw Error('Post-retirement IME key refused: '+key);}};
  await tap('KeyQ');
  const typed=await settled();
  report.foreignRangePostTyping={before:before.text,typed:typed.text,composing:typed.dom.composing};
  trace.push({label:'real key after the second task retired',settled:typed});
  if(!['q','Q'].some(letter=>typed.text===before.text+letter))throw Error('Real typing after retirement changed prior corrected or dictated text');
  if(typed.dom.composing)await tap('Escape');else await tap('Backspace');
  const cancelled=await settled();
  report.foreignRangePostTyping.afterCancel=cancelled.text;
  trace.push({label:'cancel only the new keyboard composition',settled:cancelled});
  if(cancelled.text!==before.text)throw Error('Cancelling the next keyboard composition changed prior text');
 }
}

async function runActor({page,pid,focus,start,read:readWhole,settled:settledWhole,trace,report,folder,restartBody}){
 const executable=process.env.SAYWEAVE_ACTOR_TEST_EXE;
 if(!executable||!path.isAbsolute(executable))throw Error('Built owned actor test executable required');
 const deleteWord=process.env.SAYWEAVE_ACTOR_LEARNING_CASE==='delete-word';
 const realIme=process.env.SAYWEAVE_ACTOR_LEARNING_IME==='1';
 const recordingWord=process.env.SAYWEAVE_ACTOR_LEARNING_CASE==='recording-word';
 const replaceWord=deleteWord||recordingWord||process.env.SAYWEAVE_ACTOR_LEARNING_CASE==='replace-word';
 const wordModel=recordingWord?'项目使用Sayweave，配置完成':deleteWord?(process.env.SAYWEAVE_BLOCK_FLOATING_PREDICTION==='1'?'eville':'Seville'):'项目使用Seville';
 const sentinels=process.env.SAYWEAVE_BLOCK_SENTINELS==='1';
 const sentinelPrefix=process.env.SAYWEAVE_BLOCK_SENTINEL_PREFIX;
 if(sentinelPrefix!==undefined&&(!sentinels||!sentinelPrefix||sentinelPrefix.length>64||[...sentinelPrefix].some(char=>char.codePointAt(0)<32||char.codePointAt(0)===127)))throw Error('Explicit bounded sentinel field required');
 const prefix=sentinels?(sentinelPrefix??'左|'):'',suffix=sentinels?'#右':'';
 const scoped=sample=>{
  if(!sample.text.startsWith(prefix)||!sample.text.endsWith(suffix))throw Error('Actor changed outside sentinel text');
  return {...sample,fullText:sample.text,text:sample.text.slice(prefix.length,suffix.length?-suffix.length:undefined)};
 };
 const read=async()=>scoped(await readWhole()),settled=async()=>scoped(await settledWhole());
 if(restartBody===undefined)await page.evaluate(({prefix,suffix})=>window.fixtureEditor.set(prefix+suffix,prefix.length,prefix.length),{prefix,suffix});
 else if(sentinels||(await settled()).text!==restartBody)throw Error('Restart requires the same unchanged corrected field');
 if(process.env.SAYWEAVE_BLOCK_FLOATING_PREDICTION==='1'){await page.evaluate(()=>window.fixturePrediction('示例建议 · 按 Tab 接受'));report.floatingPrediction=await page.locator('[data-owned-prediction]').count()===1;if(!report.floatingPrediction&&!sentinels)throw Error('Floating prediction fixture absent');}
 await settled();
 const initialIme=process.env.SAYWEAVE_ACTOR_INITIAL_IME==='1';
 const imeTap=async key=>{for(const down of [true,false]){const result=await focus({action:'fixture-key',process:pid,key,down});if(!result.keyResult?.accepted)throw Error('Initial IME key refused: '+key);}};
 if(initialIme){
  if(sentinels||realIme)throw Error('Initial preedit requires an independent empty-field trial');
  await page.evaluate(()=>{window.ownedInitialComposing=false;for(const type of ['compositionstart','compositionend'])document.addEventListener(type,event=>{if(event.target.closest?.('#prosemirror'))window.ownedInitialComposing=type==='compositionstart';},true);});
  let chinese=false;
  for(let attempt=0;attempt<2;attempt++){
   await imeTap('KeyQ');await settled();
   chinese=await page.evaluate(()=>window.ownedInitialComposing);
   if(chinese){await imeTap('Escape');await settled();break;}
   if(!['q','Q'].includes((await read()).text))throw Error('Initial IME mode probe changed unexpected text');
   await imeTap('Backspace');await imeTap('ShiftLeft');
  }
  if(!chinese)throw Error('Initial input did not use a real Chinese IME');
  for(const letter of 'hello')await imeTap('Key'+letter.toUpperCase());
  await settled();
  if(!await page.evaluate(()=>window.ownedInitialComposing)||(await read()).text!=='hello')throw Error('Initial foreign composition precondition missing');
  if(process.env.SAYWEAVE_ACTOR_INITIAL_IME_INTERIOR==='1')await imeTap('ArrowLeft');
 }
 const earlySwitch=initialIme&&process.env.SAYWEAVE_ACTOR_INITIAL_IME_EARLY_SWITCH==='1';
 const endSwitch=initialIme&&process.env.SAYWEAVE_ACTOR_INITIAL_IME_END_SWITCH==='1';
 if(endSwitch)await page.evaluate(()=>{
  document.addEventListener('compositionend',event=>{
   if(event.target.closest?.('#prosemirror')&&event.data==='hello'){
    document.querySelector('#input').focus();window.ownedInitialEndSwitched=true;
   }
  },true);
 });
 const refusalBarrier=path.join(folder,'initial-refusal-released');
 const caretOnly=process.env.SAYWEAVE_BLOCK_CARET_ONLY==='1';
 const rawEntry=process.env.SAYWEAVE_BLOCK_RAW_ENTRY;
 const rawMouse=['B','D-mouse'].includes(rawEntry);
 const rawModel=rawEntry?.startsWith('D-');
 if(rawEntry&&!['A','A-date','B','C','D-backspace','D-mouse','E','E-ack'].includes(rawEntry))throw Error('Unknown original-operation case');
 if(caretOnly&&(restartBody!==undefined||replaceWord||initialIme))throw Error('Caret-only continuation requires an ordinary independent block');
 const rawDelete=['E','E-ack'].includes(rawEntry);
 const controlledReply=rawEntry?(rawDelete?'新的安排。':rawMouse?'周三下午三点开会。':'下周一上午开会'):caretOnly?'周三下午三点开会。':restartBody!==undefined?'地点在上海':replaceWord?wordModel:
  process.env.SAYWEAVE_ACTOR_INITIAL_IME_DIRECT==='1'?'周三开会地点在上海':
  process.env.SAYWEAVE_BLOCK_CONTINUITY==='prediction-atomic'?'舟三开会，地点在上海。':
  ['prediction-revision','prediction-coalesced'].includes(process.env.SAYWEAVE_BLOCK_CONTINUITY)?'三开会，地点在上海。':
  process.env.SAYWEAVE_BLOCK_CONTINUITY==='delete-empty'?'新的安排。':null;
 const actor=start(executable,['--exact','dictation::drafts::tests::owned::richtext_actor','--ignored','--nocapture','--test-threads=1'],{
  SAYWEAVE_INPUT_ACCEPTANCE:'1',SAYWEAVE_TSF_OWNED_PID:String(pid),SAYWEAVE_DRAFT_STRATEGY:'revisable',
  ...(restartBody!==undefined&&process.env.SAYWEAVE_ACTOR_LEARNING_RESTART==='pending'?{SAYWEAVE_ACTOR_INITIAL_IME:'1'}:{}),
  ...(process.env.SAYWEAVE_ACTOR_PHYSICAL_INPUT==='1'?{SAYWEAVE_DATA_ACCEPTANCE_ID:randomUUID()}:{}),
  ...(controlledReply?{SAYWEAVE_ACTOR_MODEL_REPLIES:JSON.stringify(rawModel?['周三开会。',controlledReply]:caretOnly?[controlledReply,'周三下午三点开会，地点在上海。']:[controlledReply])}:{}),
  ...(earlySwitch?{SAYWEAVE_ACTOR_INITIAL_REFUSAL_BARRIER:refusalBarrier}:{}),
 });
 let buffer='',diagnostics='',exited=false,actorExit,readerStallAt,diagnosticModifierHeld=false,physicalShiftHeld=false;const values=[],pending=[];
 const diagnosticKey=process.env.SAYWEAVE_BLOCK_DIAGNOSTIC_KEY??'ControlLeft';if(!['ControlLeft','Process'].includes(diagnosticKey))throw Error('Unknown owned diagnostic key');
 actor.stderr.on('data',data=>{diagnostics+=data.toString('utf8');if(!readerStallAt&&diagnostics.includes('owned_learning_read_stall=waiting'))readerStallAt=Date.now();});
 actor.stdout.on('data',data=>{buffer+=data.toString('utf8');let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at);buffer=buffer.slice(at+1);const marker=line.indexOf('ACTOR_JSON:');if(marker<0){diagnostics+=line+'\n';continue;}const value=JSON.parse(line.slice(marker+11));if(pending.length)pending.shift().resolve(value);else values.push(value);}});
 actor.once('exit',code=>{exited=true;actorExit=code;for(const item of pending.splice(0))item.reject(Error('Actor exited '+code+': '+diagnostics));});
 const receive=()=>values.length?Promise.resolve(values.shift()):new Promise((resolve,reject)=>{if(exited){reject(Error(diagnostics));return;}const item={resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}};const timer=setTimeout(()=>{const index=pending.indexOf(item);if(index>=0)pending.splice(index,1);reject(Error('Actor response timeout: '+diagnostics));},18000);pending.push(item);});
 const command=async value=>{actor.stdin.write(JSON.stringify(value)+'\n');const reply=await receive();trace.push({label:'actor '+value.action,reply});return reply;};
 const physicalDateKey=async(key,down)=>{
  const reply=await focus({action:'fixture-key',process:pid,key,down});
  if(!reply.keyResult?.accepted)throw Error('Physical date key refused: '+key+'/'+down);
  if(key==='ShiftLeft')physicalShiftHeld=down;
 };
 const retainedProbe=process.env.SAYWEAVE_BLOCK_RETAINED_PROBE==='1';
 let retainedProbeActive=false;
 const readRetainedProbe=async label=>{const sample=await focus({action:'retained-range',mode:'read'});(report.retainedRangeProbe??=[]).push({label,...sample});return sample;};
 let lastBody='',actorFailure,actorResult,cleanupFailure,rawCleanup;const segments=[];
 const expectBody=async(label,expected)=>{
  const deadline=Date.now()+5000;let sample;
  do{sample=await settled();trace.push({label,settled:sample});if(sample.text===expected){lastBody=expected;return;}await command({action:'status'});}while(Date.now()<deadline);
  report.firstDivergence={label,kind:'actorBody',expected,actual:sample.text};throw Error(label+': coordinator body differs');
 };
 try{
  actorResult=await(async()=>{
  if(earlySwitch){
   const until=Date.now()+8000;
   while(!diagnostics.includes('owned_initial_refusal_barrier=waiting')){
    if(Date.now()>until)throw Error('The original Begin refusal barrier was not reached');
    await new Promise(resolve=>setTimeout(resolve,20));
   }
   await page.locator('#input').focus();
   writeFileSync(refusalBarrier,'released','utf8');
   trace.push({label:'same-window field changed before original refused Begin returned'});
  }
  report.actorIdentity=await receive();
  if(restartBody!==undefined){
   await expectBody('next task admission preserves the corrected word',restartBody);
   if(process.env.SAYWEAVE_ACTOR_UIA_CONFLICT==='1'){
    const admitted=await settled();
    const selection=admitted.dom.semanticSelection;
    if(!diagnostics.includes('adjacent_uia_conflict=true')||selection.start!==restartBody.length||selection.end!==restartBody.length)throw Error('Conflicting UIA evidence enlarged the empty native insertion range');
   }
   if(process.env.SAYWEAVE_ACTOR_ADJACENT_IME==='1'){
    if(report.actorIdentity.pendingInitial||!diagnostics.includes('adjacentComposition'))throw Error('Adjacent composition admission was not observed at preparation');
   }else if(process.env.SAYWEAVE_ACTOR_LEARNING_RESTART==='pending'){
    if(!report.actorIdentity.pendingInitial)throw Error('Corrected-word restart did not retain the genuine foreign composition');
    const end=Date.now()+5000;
    while(!diagnostics.includes('action=Begin applied=true')){
     if(Date.now()>end)throw Error('Corrected-word restart never completed native preparation');
     await command({action:'status'});
     await expectBody('native preparation keeps the corrected word',restartBody);
    }
   }
   await expectBody('Begin before new speech preserves the corrected word',restartBody);
   segments[0]={id:1,revision:1,text:'地点',confirmed:false};
   await command({action:'asr',segments});
   await expectBody('first new source preserves the corrected word',restartBody+'地点');
   segments[0]={id:1,revision:2,text:'地点在上海',confirmed:true};
   await command({action:'asr',segments});
   await expectBody('continued source preserves the corrected word',restartBody+'地点在上海');
   report.actorDelivery=await command({action:'finish'});
   await expectBody('next task final body preserves the corrected word',restartBody+'地点在上海');
   if(report.actorDelivery.error||report.actorDelivery.resultOnly||report.actorDelivery.requests!==1)throw Error('Corrected-word restart did not finish live input and processing');
   await command({action:'done'});
   return;
  }
  if(initialIme){
   if(process.env.SAYWEAVE_ACTOR_ADJACENT_IME==='1'){
    if(report.actorIdentity.pendingInitial||!diagnostics.includes('adjacentComposition'))throw Error('Adjacent composition admission was not observed at preparation');
   }else if(!report.actorIdentity.pendingInitial)throw Error('Begin did not observe the genuine initial foreign composition');
   segments[0]={id:1,revision:1,text:'周',confirmed:false};
   await command({action:'asr',segments});
   segments[0]={id:1,revision:2,text:'周三',confirmed:false};
   const pendingStatus=await command({action:'asr',segments});
   if(endSwitch)await page.waitForFunction(()=>window.ownedInitialEndSwitched===true,null,{timeout:5000});
   if(!earlySwitch)await expectBody('foreign IME text retained while recognition progresses',process.env.SAYWEAVE_ACTOR_INITIAL_IME_DIRECT==='1'&&!endSwitch?'hello周三':'hello');
   if(pendingStatus.audioRoute!==1||(!earlySwitch&&!endSwitch&&pendingStatus.resultOnly)||pendingStatus.presentation!=='recording')throw Error('Pending input changed the recognition route');
   const switchField=earlySwitch||endSwitch||process.env.SAYWEAVE_ACTOR_INITIAL_IME_SWITCH==='1';
   const expire=process.env.SAYWEAVE_ACTOR_INITIAL_IME_TIMEOUT==='1';
   if(switchField||expire){
    if(switchField&&!earlySwitch&&!endSwitch)await page.locator('#input').focus();
    const deadline=Date.now()+(expire?20000:6000);let state;
    do{await new Promise(resolve=>setTimeout(resolve,150));state=await command({action:'status'});}while(!state.resultOnly&&Date.now()<deadline);
    report.initialImeOtherBody=await page.locator('#input').inputValue();
    if(report.initialImeOtherBody!=='')throw Error('A different field received pending source');
    if(!state.resultOnly||state.audioRoute!==1)throw Error('Pending refusal did not preserve recording in result delivery');
    segments[0]={id:1,revision:3,text:'周三开会',confirmed:true};
    await command({action:'asr',segments});
    report.actorDelivery=await command({action:'finish'});
    if(!report.actorDelivery.resultOnly||report.actorDelivery.resultRaw!=='周三开会')throw Error('Pending result delivery lost the recognition source');
    report.initialImeBoundary=earlySwitch?'different-field-before-refusal':endSwitch?'different-field-during-ime-completion':switchField?'different-field':'deadline';report.actorPassed=true;
    return;
   }
   if(process.env.SAYWEAVE_ACTOR_INITIAL_IME_DIRECT==='1'){
    report.initialImeBoundary='direct-dictation-without-commit-key';
    segments[0]={id:1,revision:3,text:'周三开会',confirmed:true};
    await command({action:'asr',segments});
    await expectBody('direct speech must progress without fixture committing the IME','hello周三开会');
    segments.push({id:2,revision:1,text:'地点在上海',confirmed:true});
    await command({action:'asr',segments});
    await expectBody('second speech segment after uncommitted keyboard edit','hello周三开会地点在上海');
    report.actorDelivery=await command({action:'finish'});
    await expectBody('direct initial IME final actual body','hello周三开会地点在上海');
    if(report.actorDelivery.resultOnly||report.actorDelivery.error||report.actorDelivery.requests!==1)throw Error('Direct initial IME speech did not finish live input and processing');
    report.initialImePassed=true;report.actorPassed=true;
    return;
   }
   await imeTap('Space');await settled();
   if(await page.evaluate(()=>window.ownedInitialComposing))throw Error('Original IME remained active after explicit fixture commit');
   const committedPrefix=(await read()).text;
   if(!committedPrefix)throw Error('IME commit removed its original pending text');
   report.initialImeCommittedPrefix=committedPrefix;
   segments[0]={id:1,revision:3,text:'周三开会',confirmed:true};
   await command({action:'asr',segments});await expectBody('pending first source delivered once after IME commit',committedPrefix+'周三开会');
   segments[0]={id:1,revision:4,text:'周三开会地点上海',confirmed:true};
   await command({action:'asr',segments});await expectBody('continued stream after pending initial input',committedPrefix+'周三开会地点上海');
   report.actorDelivery=await command({action:'finish'});
   await expectBody('initial IME final actual body',committedPrefix+'周三开会地点上海');
   if(report.actorDelivery.resultOnly||report.actorDelivery.error)throw Error('Initial IME trial did not finish live input');
   report.initialImePassed=true;report.actorPassed=true;
   return;
  }
  if(process.env.SAYWEAVE_ACTOR_LEARNING==='1'&&process.env.SAYWEAVE_ACTOR_LEARNING_CASE!=='regression'){
   const resumed=process.env.SAYWEAVE_ACTOR_LEARNING_CASE==='resume';
   const wrong=replaceWord?'Seville':'SQLit',correct=realIme?'sayweave':replaceWord?'Sayweave':'SQLite';
   const before=deleteWord?'Seville':replaceWord?'项目使用Seville':resumed?'应用使用SQLit，配置完成':'项目使用SQLit';
   let after=before.replace(wrong,correct);
   report.learningProvider=process.env.SAYWEAVE_ACTOR_LEARNING_FIELD==='1'?'original-field':'native-and-field';
   segments[0]={id:1,revision:1,text:replaceWord?before:'项目使用SQLit',confirmed:!resumed&&!recordingWord};
   if(deleteWord&&process.env.SAYWEAVE_BLOCK_FLOATING_PREDICTION==='1'){
    segments[0]={id:1,revision:1,text:'S',confirmed:false};
    await command({action:'asr',segments});await expectBody('first letter before cancelled scope','S');
    segments[0]={id:1,revision:2,text:before,confirmed:true};
   }
   await command({action:'asr',segments});
   await expectBody('learning actual delivered body',replaceWord?before:'项目使用SQLit');
   if(resumed){
    const bindEnd=Date.now()+5000;
    while(!diagnostics.includes('operation=draft_block_bound')){
     if(Date.now()>bindEnd)throw Error('Learning initial block not bound');
     await command({action:'status'});await settled();
    }
    await command({action:'edited'});
    for(const down of [true,false])await focus({action:'fixture-key',process:pid,key:'ArrowLeft',down});
    await settled();
    await page.evaluate(()=>{window.fixtureEditor.edit(0,2,'应用');window.fixtureEditor.select(9);});
    await expectBody('learning manual edit before resume','应用使用SQLit');
    segments[0].confirmed=true;
    segments.push({id:2,revision:1,text:'，配置完成',confirmed:true});
    await command({action:'asr',segments});
    await expectBody('learning reacquired and polished body',before);
   }
   if(!recordingWord){
    report.actorDelivery=await command({action:'finish'});
    report.learningFirstFinishedAt=Date.now();
    await expectBody('learning body after finish',before);
    if(report.actorDelivery.error||report.actorDelivery.resultOnly||report.actorDelivery.delivery?.restoreError)throw Error('Learning input not safely delivered');
   }else{
    // Keep the production recording task alive and hand the real composition
    // to native editing before changing the word.
    await command({action:'edited'});
    for(const down of [true,false])await focus({action:'fixture-key',process:pid,key:'ArrowLeft',down});
    await settled();
    await page.evaluate(at=>window.fixtureEditor.select(at),prefix.length+before.length);
   }
   if(resumed)await page.evaluate(()=>window.fixtureEditor.select('应用使用SQLit'.length));
   if(deleteWord){
    for(let index=0;index<wrong.length;index++){
     for(const down of [true,false])await focus({action:'fixture-key',process:pid,key:'Backspace',down});
     await expectBody('delete word character '+(index+1),before.slice(0,-index-1));
    }
    await new Promise(resolve=>setTimeout(resolve,1200));
   }else if(replaceWord){
    await focus({action:'fixture-key',process:pid,key:'ShiftLeft',down:true});
    try{for(let index=0;index<wrong.length;index++)for(const down of [true,false])await focus({action:'fixture-key',process:pid,key:'ArrowLeft',down});}
    finally{await focus({action:'fixture-key',process:pid,key:'ShiftLeft',down:false});}
    await settled();
   }
   if(realIme){
    if(!deleteWord||sentinels||recordingWord)throw Error('Real IME correction requires the whole-field delete-word fixture');
    await page.evaluate(()=>{window.ownedLearningComposing=false;for(const type of ['compositionstart','compositionend'])document.addEventListener(type,event=>{if(event.target.closest?.('#prosemirror'))window.ownedLearningComposing=type==='compositionstart';},true);});
    const tap=async key=>{for(const down of [true,false]){const result=await focus({action:'fixture-key',process:pid,key,down});if(!result.keyResult?.accepted)throw Error('Real IME fixture key refused: '+key);}};
    let chinese=false;
    for(let attempt=0;attempt<2;attempt++){
     await tap('KeyQ');await settled();
     chinese=await page.evaluate(()=>window.ownedLearningComposing);
     if(chinese){
      await tap('Escape');await settled();
      const probe=(await read()).text;
      if(probe==='q'||probe==='Q')await tap('Backspace');
      await expectBody('cancel mode probe in original empty field','');break;
     }
     await expectBody('English mode probe','q');await tap('Backspace');await tap('ShiftLeft');
    }
    if(!chinese)throw Error('Actual Chinese IME preedit was not established');
    for(const letter of correct)await tap('Key'+letter.toUpperCase());
    await expectBody('actual IME pending correction',after);
    if(!await page.evaluate(()=>window.ownedLearningComposing))throw Error('IME correction unexpectedly bypassed composition');
    const restart=process.env.SAYWEAVE_ACTOR_LEARNING_RESTART;
    if(restart){
     if(!['pending','committed','shifted'].includes(restart))throw Error('Explicit correction restart state required');
     if(restart==='committed'||restart==='shifted'){
      const commitKey=restart==='shifted'?'ShiftLeft':'Space';
      if(restart==='shifted'){
       await page.evaluate(()=>{window.ownedCorrectionCommitEvents=[];for(const type of ['keydown','keyup','compositionend'])document.addEventListener(type,event=>{if(event.target.closest?.('#prosemirror'))window.ownedCorrectionCommitEvents.push({type,key:event.key,code:event.code,data:event.data,body:window.fixtureEditor.read()});},true);});
       const down=await focus({action:'fixture-key',process:pid,key:commitKey,down:true});
       if(!down.keyResult?.accepted)throw Error('Correction Shift down refused');
       let up;
       try{await new Promise(resolve=>setTimeout(resolve,250));}
       finally{up=await focus({action:'fixture-key',process:pid,key:commitKey,down:false});}
       if(!up.keyResult?.accepted)throw Error('Correction Shift up refused');
      }else await tap(commitKey);
      const deadline=Date.now()+3000;
      do{await settled();if(!await page.evaluate(()=>window.ownedLearningComposing))break;}while(Date.now()<deadline);
      if(restart==='shifted')report.correctionCommitEvents=await page.evaluate(()=>window.ownedCorrectionCommitEvents);
      if(await page.evaluate(()=>window.ownedLearningComposing))throw Error('Correction restart IME was not committed');
      const committed=(await read()).text;
      if(committed!==after&&(restart!=='committed'||committed!==after+' '))throw Error('Committing correction changed its spelling');
      after=committed;
      report.correctionCommit={key:commitKey,body:after,composing:false};
     }
     await new Promise(resolve=>setTimeout(resolve,Math.max(0,8000-(Date.now()-report.learningFirstFinishedAt))));
     await expectBody('corrected word before the next task',after);
     report.correctionRestart={state:restart,body:after,afterFinishMs:Date.now()-report.learningFirstFinishedAt};
     await command({action:'done'});
     return after;
    }
    const quietEnd=Date.now()+15500;
    while(Date.now()<quietEnd){await new Promise(resolve=>setTimeout(resolve,1000));await command({action:'status'});}
    let stable=await command({action:'learning'});
    const settleEnd=Date.now()+6000;
    while(!stable.learning.some(row=>row.observed.after===after)&&Date.now()<settleEnd){await new Promise(resolve=>setTimeout(resolve,200));stable=await command({action:'learning'});}
    if(!stable.learning.some(row=>row.observed.after===after))throw Error('Stable original-field edit was gated by composition metadata');
    if(!diagnostics.includes('composition_range=Some(true)'))throw Error('Readonly diagnostic did not observe the cached composition range');
    report.actualIme={stableBody:after,stableSnapshotLearned:stable.learning.length,compositionRangeStillPresent:true};
    await tap('Space');await settled();
    if(await page.evaluate(()=>window.ownedLearningComposing))throw Error('Actual IME did not commit the correction');
    const committed=(await read()).text;
    if(committed===after+' ')after=committed;
    report.actualIme.committed=true;
   }else{
    const typed=await focus({action:'fixture-text',process:pid,text:replaceWord?correct:'e'});
    if(typed.sent!==typed.expected)throw Error('Lexical correction key injection incomplete');
   }
   await expectBody('actual keyboard lexical correction',after);
   const submission=process.env.SAYWEAVE_ACTOR_LEARNING_SUBMIT;
   if(submission){
    if(submission!=='enter'||realIme||sentinels)throw Error('Submission fixture requires a whole-field keyboard correction');
    await new Promise(resolve=>setTimeout(resolve,1200));
    // This is the owned app's normal submit action. Real native Enter drives
    // it; the learning observer never clears or changes the editor.
    await page.evaluate(()=>{
     document.querySelector('#prosemirror').addEventListener('keydown',event=>{
      if(event.key==='Enter'&&!event.isComposing){event.preventDefault();window.fixtureEditor.set('',0,0);window.ownedLearningSubmitted=true;}
     });
    });
    const submittedAt=Date.now();
    for(const down of [true,false])await focus({action:'fixture-key',process:pid,key:'Enter',down});
    await expectBody('app submitted correction before idle','');
    if(!await page.evaluate(()=>window.ownedLearningSubmitted===true))throw Error('App did not handle the real submission key');
    report.learningSubmission={kind:submission,submittedAt,bodyAfterSubmit:''};
   }
   if(recordingWord&&!submission){
    const captureEnd=Date.now()+5000;
    while(!diagnostics.includes('result=correctionCaptured')){
     if(Date.now()>captureEnd)throw Error('Recording edit was not captured before continuation');
     await command({action:'status'});await settled();
    }
    segments[0].confirmed=true;
    segments.push({id:2,revision:1,text:'，配置完成',confirmed:true});
    await command({action:'asr',segments});
    await expectBody('continued speech after recorded lexical edit',after+'，配置完成');
   }
   const end=Date.now()+(submission?6000:22000);
   do{
    report.learning=await command({action:'learning'});
    if(report.learning.learning.some(row=>row.observed.after===after))break;
    await new Promise(resolve=>setTimeout(resolve,200));
   }while(Date.now()<end);
   const learned=report.learning.learning.filter(row=>row.observed.after===after);
   for(const row of report.learning.learning.filter(row=>row.observed.after!==after)){
    if(deleteWord&&row.observed.before==='eville'&&'eville'.startsWith(row.observed.after)&&row.observed.after.length<6){
     if(row.class!=='deletion'||row.terms.some(term=>term.original!==correct))throw Error('Transient native deletion created an automatic word');
     continue;
    }
    if(!resumed||row.observed.before!=='项目使用SQLit'||row.observed.after!=='应用使用SQLit'||row.terms.length!==0)throw Error('Own resumed write was mistaken for a user correction');
   }
   if(!learned.length)throw Error('Actual correction produced no learning evidence');
   if(new Set(learned.map(row=>row.observed.id)).size!==1)throw Error('Readers produced independent identities for one correction');
   if(learned.reduce((count,row)=>count+row.classifierRequests,0)!==1)throw Error('Correction classified more than once');
   for(const row of learned){
    if(row.observed.before!==before||row.observed.after!==after)throw Error('Learning scope differs from actual output');
    if(row.terms.length!==1||row.terms[0].original!==correct||!row.nextHotwords.includes(correct)||!row.nextPromptContainsTerm)throw Error('Learning term/next input snapshot differs');
   }
   if(recordingWord){
    report.actorDelivery=await command({action:submission?'stop':'finish'});
    await expectBody('recording correction final body',submission?'':after+'，配置完成');
    if(!submission&&(report.actorDelivery.error||report.actorDelivery.resultOnly))throw Error('Recording correction broke input delivery');
   }
   await command({action:'done'});
   report.actorPassed=true;report.learningPassed=true;
   return;
  }
  if(rawEntry){
   if(sentinels||restartBody!==undefined||initialIme||caretOnly)throw Error('Original operation needs one genuinely empty original field');
   const triggerBody=rawModel?'周三开会。':'周三开会';
   const beforeLog=diagnostics.length;
   const pointerWindow=rawMouse?await focus({action:'read'}):null;
   if(pointerWindow&&pointerWindow.foregroundProcess!==pid)throw Error('Original pointer field lost focus before input');
   const binding='ownedRaw'+randomUUID().replaceAll('-','');
   let resolveStimulus,rejectStimulus,timer,rawActive=true,rawDispatch;
   const pressed=new Set();
   const stimulus=new Promise((resolve,reject)=>{resolveStimulus=resolve;rejectStimulus=reject;});stimulus.catch(()=>{});
   const nativeTrigger=['A','A-date'].includes(rawEntry);
   const evidence=report.rawEntry={case:rawEntry,nativePrewarm:false,triggerKind:nativeTrigger?'native first-write receipt':'renderer input event',preEditReads:nativeTrigger?[]:['DOM input-event text','pointer geometry only when needed'],actions:[],triggerBody};
   const admit=()=>{if(!rawActive)throw Error('Original stimulus retired');};
   const release=async key=>{await physicalDateKey(key,false);pressed.delete(key);evidence.actions.push({key,down:false,at:Date.now()});};
   rawCleanup=async()=>{
    rawActive=false;clearTimeout(timer);
    const failures=[];
    try{await page.evaluate(binding=>{const state=window[binding+'State'];if(state){state.active=false;document.removeEventListener('input',state.listener,true);delete window[binding+'State'];}},binding);}catch(error){failures.push(String(error));}
    if(rawDispatch){let deadline;try{await Promise.race([rawDispatch.catch(()=>{}),new Promise((_,reject)=>{deadline=setTimeout(()=>reject(Error('Original stimulus cleanup deadline')),5500);})]);}catch(error){failures.push(String(error));}finally{clearTimeout(deadline);}}
    for(const key of pressed)try{await release(key);}catch(error){failures.push(String(error));}
    evidence.cleanup={at:Date.now(),pendingKeys:[...pressed],errors:failures};
    if(failures.length)throw Error(failures.join('; '));
   };
   const dispatchStimulus=event=>{
    if(!rawActive)return;
    rawDispatch=(async()=>{
    try{
     evidence.trigger=event;evidence.dispatchedAt=Date.now();
     const tap=async key=>{admit();pressed.add(key);try{await physicalDateKey(key,true);evidence.actions.push({key,down:true,at:Date.now()});}finally{await release(key);}};
     if(rawMouse){
      const scale=pointerWindow.foregroundClient[0]/event.point.innerWidth;
      admit();
      const click=await focus({action:'click-window',process:pid,window:pointerWindow.foregroundWindow,x:Math.round(event.point.x*scale),y:Math.round(pointerWindow.foregroundClient[1]-event.point.innerHeight*scale+event.point.y*scale)});
      if(click.clicked!==2)throw Error('Original pointer stimulus rejected');
      evidence.actions.push({mouse:click,at:Date.now()});
     }else if(rawEntry==='C'){
      for(const key of ['ArrowLeft','ArrowLeft','Backspace'])await tap(key);
      admit();
      const typed=await focus({action:'fixture-text',process:pid,text:'五'});
      if(!typed.keyResult?.accepted)throw Error('Original date correction rejected');
      evidence.actions.push({text:'五',at:Date.now()});
     }else if(rawDelete){
      evidence.deleteStimulus=rawEntry==='E'?'four-packet burst; delivery-count diagnostic':'four held repeats acknowledged by host text; not hardware-repeat timing';
      try{for(let count=0;count<4;count++){
       admit();pressed.add('Backspace');await physicalDateKey('Backspace',true);evidence.actions.push({key:'Backspace',down:true,at:Date.now()});
       if(rawEntry==='E-ack'){
        // No pre-read or handoff wait before the first Down. Each of the four
        // held repeats must remove exactly one character; never send extras.
        const expected=triggerBody.slice(0,3-count);
        await page.waitForFunction(text=>window.fixtureEditor.read()===text,expected,{timeout:2000});
        evidence.actions.push({hostDeletion:expected,at:Date.now()});
       }
      }}
      finally{await release('Backspace');}
     }else{
      await tap('Backspace');
      if(rawEntry==='A-date'){
       await page.waitForFunction(()=>window.fixtureEditor.read()==='周三开',undefined,{timeout:2000});
       evidence.actions.push({firstBackspaceBody:'周三开',at:Date.now()});
       admit();pressed.add('ControlLeft');
       try{await physicalDateKey('ControlLeft',true);evidence.actions.push({key:'ControlLeft',down:true,at:Date.now()});await tap('KeyA');}
       finally{await release('ControlLeft');}
       admit();const typed=await focus({action:'fixture-text',process:pid,text:'周五开会'});
       if(!typed.keyResult?.accepted)throw Error('Original correction after first Backspace rejected');
       evidence.actions.push({text:'周五开会',at:Date.now()});
      }
     }
     resolveStimulus();
    }catch(error){rejectStimulus(error);}
    })();
    return rawDispatch;
   };
   if(!nativeTrigger){
   await page.exposeBinding(binding,(_,event)=>dispatchStimulus(event));
   await page.evaluate(({binding,triggerBody,mouse})=>{
    let fired=false;
    const state={active:true,listener:null};window[binding+'State']=state;
    state.listener=event=>{
     if(!state.active||fired||!event.target.closest?.('#prosemirror'))return;
     queueMicrotask(()=>{
      if(!state.active||fired||window.fixtureEditor.read()!==triggerBody)return;
      fired=true;const sample={body:window.fixtureEditor.read(),event:event.type,inputType:event.inputType,hostAt:performance.now()};
      if(mouse){
       const walker=document.createTreeWalker(document.querySelector('#prosemirror'),NodeFilter.SHOW_TEXT),text=walker.nextNode();
       const range=document.createRange();range.setStart(text,2);range.setEnd(text,3);const rect=range.getBoundingClientRect();
       sample.point={x:rect.left+1,y:rect.top+rect.height/2,innerWidth,innerHeight};
      }
      window[binding](sample).catch(()=>{});
     });
    };document.addEventListener('input',state.listener,true);
   },{binding,triggerBody,mouse:rawMouse});
   }
   segments.push({id:1,revision:1,text:'周三开会',confirmed:rawModel});
   try{
    if(rawEntry==='C'){
     for(const text of ['周','周三']){segments[0].text=text;await command({action:'asr',segments});await page.waitForFunction(text=>window.fixtureEditor.read()===text,text,{timeout:5000});evidence.actions.push({observedDOM:text,at:Date.now()});segments[0].revision++;}
     segments[0].text='周三开会';
    }
    await command({action:'asr',segments});
    if(nativeTrigger){
     const deadline=Date.now()+8000;let receipt;
     while(!(receipt=diagnostics.slice(beforeLog).split('\n').find(line=>line.includes(`operation=composition_edit task_id=${report.actorIdentity.task} `)&&line.includes('turn=1 revision=1 action=Update applied=true')&&line.includes('mutation=Changed')))){
      if(Date.now()>=deadline)throw Error('Exact first native write receipt absent; no edit sent');
      await new Promise(resolve=>setTimeout(resolve,5));
     }
     void dispatchStimulus({nativeReceipt:receipt,at:Date.now(),postWriteTextReadBeforeFirstEdit:false});
    }
    await Promise.race([stimulus,new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('Original stimulus input event not reached')),8000);})]);
   }finally{clearTimeout(timer);}
   await rawCleanup();rawCleanup=null;
   const edited=rawDelete?'':['C','A-date'].includes(rawEntry)?'周五开会':rawEntry==='A'?'周三开':rawMouse?triggerBody:'周三开会';
   await expectBody('original immediate edit '+rawEntry,edited);
   if(rawModel&&!diagnostics.slice(beforeLog).includes('operation=draft_candidate'))throw Error('Model result was not actually applied before the editing event');
   if(rawMouse){const sample=await read();if(sample.dom.semanticSelection.start!==2||sample.dom.semanticSelection.end!==2)throw Error('Original click did not reach middle');}
   segments[0].confirmed=true;
   segments.push({id:2,revision:1,text:rawDelete?'新的安排':rawMouse?'下午三点':'，不对，改成下周一上午开会',confirmed:true});
   await command({action:'asr',segments});await expectBody('original whole result '+rawEntry,controlledReply);
   if(!rawDelete&&diagnostics.slice(beforeLog).includes('operation=draft_block_sealed'))throw Error('Original whole result lost its initial block');
   evidence.sourceFlow=diagnostics.slice(beforeLog).split('\n').filter(line=>line.includes('receiptFlow='));
   if(!rawDelete&&process.env.SAYWEAVE_BLOCK_REQUIRE_NATIVE_RECEIPT==='1'&&!evidence.sourceFlow.some(line=>line.includes(`task_id=${report.actorIdentity.task} `)&&/\bturn=1\b/.test(line)&&/sourceReceipt=committed(?:\.handoff)?\//.test(line)&&line.includes('nativeReceipt.paired')))throw Error('Correct body did not commit the required original-task native receipt proof');
   report.actorDelivery=await command({action:'finish'});
   if(report.actorDelivery.error||report.actorDelivery.resultOnly)throw Error('Original operation did not finish direct input');
   await expectBody('original final body '+rawEntry,controlledReply);
   if(process.env.SAYWEAVE_ACTOR_LEARNING==='1'||process.env.SAYWEAVE_ACTOR_KEEP_AFTER_FINISH==='1')await command({action:'done'});
   actor.stdin.end();
   if(!exited)await new Promise((resolve,reject)=>{const timeout=setTimeout(()=>reject(Error('Original actor cleanup deadline')),5000);actor.once('exit',code=>{clearTimeout(timeout);if(code===0)resolve();else reject(Error('Original actor exit '+code));});});
   evidence.passed=true;report.actorPassed=true;return;
  }
  const continuity=process.env.SAYWEAVE_BLOCK_CONTINUITY;
  const literalCaret=process.env.SAYWEAVE_BLOCK_LITERAL_CARET==='1';
  if(literalCaret&&continuity!=='delete-empty')throw Error('Literal continuation expectation is limited to the explicit retired-empty-block scenario');
  if(continuity){
   if(!['prediction','prediction-revision','prediction-coalesced','prediction-atomic','prediction-switch','delete-empty'].includes(continuity)||sentinels)throw Error('Explicit empty-field continuity scenario required');
   report.continuityScenario=continuity;
   if(continuity==='prediction-switch'){
    if(process.env.SAYWEAVE_BLOCK_SEAL_BARRIER!=='1')throw Error('Release barrier required');
    segments[0]={id:1,revision:1,text:'周',confirmed:false};
    await command({action:'asr',segments});await expectBody('before scope switch','周');
    segments[0]={id:1,revision:2,text:'周三',confirmed:false};
    await command({action:'asr',segments});
    let deadline=Date.now()+5000;
    while(!diagnostics.includes('owned_bind_barrier=waiting')){
     if(Date.now()>deadline)throw Error('Settled scope release barrier absent');
     await command({action:'status'});await settled();
    }
    await page.evaluate(()=>window.fixtureInlineEditor.set('另一个输入框',6,6));
    await command({action:'release-bind'});
    let status;deadline=Date.now()+5000;
    do{status=await command({action:'status'});if(Date.now()>deadline)throw Error('Fresh scope did not write the unconsumed suffix');await settled();}while(await page.evaluate(()=>window.fixtureInlineEditor.read())!=='另一个输入框三');
    segments[0]={id:1,revision:3,text:'周三开会',confirmed:false};
    await command({action:'asr',segments});
    deadline=Date.now()+5000;
    while(await page.evaluate(()=>window.fixtureInlineEditor.read())!=='另一个输入框三开会'){
     if(Date.now()>deadline)throw Error('Fresh scope stopped making body progress');
     status=await command({action:'status'});await settled();
    }
    report.actorDelivery=await command({action:'finish'});
    const bodies=await page.evaluate(()=>({original:window.fixtureEditor.read(),other:window.fixtureInlineEditor.read()}));
    if(bodies.original!=='周'||bodies.other!=='另一个输入框三开会'||report.actorDelivery.resultOnly)throw Error('Fresh scope replayed old text or stopped recognition: '+JSON.stringify(bodies));
    report.scopeSwitch={...bodies,audioRoute:status.audioRoute,resultRaw:report.actorDelivery.resultRaw};
    if(status.audioRoute!==1)throw Error('Field change stopped recognition audio');
    actor.stdin.end();
    if(!exited)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Scope-switch cleanup timeout')),5000);actor.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Scope-switch actor failed: '+diagnostics));});});
    report.actorPassed=true;return;
   }
   if(['prediction-revision','prediction-coalesced','prediction-atomic'].includes(continuity)){
    if(process.env.SAYWEAVE_BLOCK_FLOATING_PREDICTION!=='1')throw Error('The original empty prediction scene is required');
    const atomic=continuity==='prediction-atomic';
    const steps=atomic?[[1,'周','周'],[2,'舟三开会','舟三开会']]:[[1,'周','周'],[2,'周三','周三'],[3,'舟三开会','周三开会']];
    const prefix=atomic?'舟三开会':'周三开会';
    if(process.env.SAYWEAVE_BLOCK_DIAGNOSTIC_HOLD==='1'){
     const held=await focus({action:'fixture-key',process:pid,key:diagnosticKey,down:true});
     if(!held.keyResult?.accepted)throw Error('Owned diagnostic modifier not dispatched');
     diagnosticModifierHeld=true;report.diagnosticHeldModifier=true;
     if(process.env.SAYWEAVE_BLOCK_DIAGNOSTIC_DELAY==='1'){await new Promise(resolve=>setTimeout(resolve,200));report.diagnosticDelayMs=200;}
    }
    if(continuity==='prediction-coalesced'){
     if(process.env.SAYWEAVE_BLOCK_SEAL_BARRIER!=='1')throw Error('Original scope release barrier required');
     segments[0]={id:1,revision:1,text:'周',confirmed:false};
     await command({action:'asr',segments});await expectBody('coalesced first actual body','周');
     segments[0]={id:1,revision:2,text:'周三',confirmed:false};
     await command({action:'asr',segments});
     const deadline=Date.now()+5000;
     while(!diagnostics.includes('owned_bind_barrier=waiting')){
      if(Date.now()>deadline)throw Error('Original release barrier not reached');
      await command({action:'status'});await settled();
     }
     for(const [revision,text]of [[3,'周三开会'],[4,'舟三开会']]){
      segments[0]={id:1,revision,text,confirmed:false};
      await command({action:'asr',segments});
     }
     await command({action:'release-bind'});
     await expectBody('coalesced source still advances actual body','周三开会');
    }else for(const [revision,text,expected]of steps){
     segments[0]={id:1,revision,text,confirmed:false};
     await command({action:'asr',segments});await expectBody('prediction revised-source stream '+revision,expected);
     if(revision===1&&diagnosticModifierHeld){
      const released=await focus({action:'fixture-key',process:pid,key:diagnosticKey,down:false});
      if(!released.keyResult?.accepted)throw Error('Owned diagnostic modifier release refused');
      diagnosticModifierHeld=false;
     }
    }
    segments.push({id:2,revision:1,text:'地点在上海',confirmed:false});
    await command({action:'asr',segments});
    await expectBody('prediction must still write the next speech segment',prefix+'地点在上海');
    report.actorDelivery=await command({action:'finish'});
    await expectBody('prediction revised-source final body',prefix+'，地点在上海。');
    if(report.actorDelivery.resultOnly||report.actorDelivery.error)throw Error('Prediction source revision silently detached live input');
    report.actorPassed=true;return;
   }
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
    if(process.env.SAYWEAVE_BLOCK_IDLE_TYPED_PREFIX==='1'){
     const deadline=Date.now()+5000;
     while(!diagnostics.includes('operation=draft_block_sealed')){
      if(Date.now()>deadline)throw Error('Deleted old scope did not settle');
      await command({action:'status'});await settled();
     }
     const typed=await focus({action:'fixture-text',process:pid,text:'手改：'});
     if(!typed.keyResult?.accepted)throw Error('Idle manual prefix injection failed');
     await expectBody('idle manual prefix before new speech','手改：');
     if((diagnostics.match(/action=Begin applied=true/g)??[]).length!==1)throw Error('An empty native range was reopened before new speech');
     report.idleManualPrefix='手改：';
    }
    segments[0].confirmed=true;
    segments.push({id:2,revision:1,text:'新的安排',confirmed:true});
    await command({action:'asr',segments});await expectBody('continuity after delete',(report.idleManualPrefix??'')+(literalCaret?'新的安排':'新的安排。'));
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
   const final=continuity==='prediction'?'周五开会，地点在上海。':(report.idleManualPrefix??'')+(literalCaret?'新的安排':'新的安排。');
   await expectBody('continuity final settled',final);
   if(literalCaret){
    const mode=diagnostics.split('\n').find(line=>line.includes('operation=draft_input_mode task_id='+report.actorIdentity.task+' ')&&line.includes('mode=caret'));
    if(!mode||report.actorDelivery.finished!==true||report.actorDelivery.delivery?.submission!=='submitted'||report.actorDelivery.requests!==0||!diagnostics.includes('operation=draft_block_sealed task_id='+report.actorIdentity.task+' '))throw Error('Literal empty-block continuation did not prove sealing, caret mode and completed direct delivery without model requests');
    report.literalCaret={mode,modelRequests:report.actorDelivery.requests};
   }
   if(report.actorDelivery.error||report.actorDelivery.resultOnly||report.actorDelivery.delivery?.restoreError)throw Error('Continuity finished without safe direct input');
   if(process.env.SAYWEAVE_ACTOR_LEARNING==='1')await command({action:'done'});
   actor.stdin.end();
   if(!exited)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Continuity cleanup timeout')),5000);actor.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Continuity actor failed: '+diagnostics));});});
   if(literalCaret&&!diagnostics.split('\n').some(line=>line.includes('operation=composition_retired task_id='+report.actorIdentity.task+' ')&&line.includes('phase=resourcesReleased result=complete error=none')))throw Error('Exact literal task native resources did not retire');
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
  const keyboardEdit=process.env.SAYWEAVE_BLOCK_KEYBOARD_EDIT==='1';
  const physicalDate=process.env.SAYWEAVE_BLOCK_PHYSICAL_DATE==='1';
  const handoffBarrier=process.env.SAYWEAVE_BLOCK_HANDOFF_BARRIER==='1';
  if(handoffBarrier&&(!physicalDate||earlyEdit||commitEarly))throw Error('Handoff barrier requires the real keyboard date route');
  if(physicalDate&&(!keyboardEdit||backspace||earlyEdit||commitEarly||interiorEdit||sentinels||initialIme||firstOnly||process.env.SAYWEAVE_BLOCK_SELECTION_PAUSE==='1'))throw Error('Physical date regression requires an empty field and ordinary keyboard-edit path');
  report.actorKeyboardEdits=keyboardEdit;
  if(interiorEdit&&(backspace||earlyEdit||commitEarly))throw Error('Interior-edit round trip requires the ordinary bound block');
  report.actorInteriorEditRoundTrip=interiorEdit;
  const userNewline=process.env.SAYWEAVE_BLOCK_USER_NEWLINE==='1';
  if(userNewline&&!backspace)throw Error('User newline regression requires direct Backspace setup');
  if(userNewline)report.actorNewlineRepresentation='paragraph';
  const retained=userNewline?'\n':'';
  const firstText=backspace?'周三开会。':'周三开会';
  if(retainedProbe)(report.retainedRangeProbe??=[]).push({label:'before first native output',...await focus({action:'retained-range',mode:'inspect',process:pid})});
  const originProbe=retainedProbe&&process.env.SAYWEAVE_BLOCK_RETAINED_ORIGIN==='1';
  if(originProbe)(report.retainedOriginProbe??=[]).push({label:'before first native output',...await focus({action:'retained-range',mode:'capture-origin',process:pid,expected:'\n'})});
  const originCase=process.env.SAYWEAVE_BLOCK_ORIGIN_CASE;
  if(originCase){
   if(!['prepared-character','prepared-rebuild','paragraph-end'].includes(originCase))throw Error('Unknown retained-origin diagnostic');
   report.originDiagnostic={case:originCase};
   if(originCase==='prepared-character')await page.evaluate(()=>window.fixtureEditor.edit(0,0,'外'));
   if(originCase==='prepared-rebuild')await page.evaluate(()=>window.fixtureEditor.rebuildParagraph());
   if(originCase==='paragraph-end')await page.evaluate(()=>document.addEventListener('compositionend',event=>{
    if(!window.ownedOriginRebuilt&&event.target.closest?.('#prosemirror')){window.ownedOriginRebuilt=true;window.fixtureEditor.rebuildParagraph();}
   },true));
   report.originDiagnostic.beforeInput=await settled();
  }
  segments.push({id:1,revision:1,text:firstText,confirmed:false});
  if(process.env.SAYWEAVE_BLOCK_INITIAL_STREAM==='1'){
   if(earlyEdit)throw Error('Initial streaming must not bypass the first-write edit barrier');
   for(const text of ['周','周三','周三开']){
    segments[0].text=text;segments[0].revision++;
    await command({action:'asr',segments});await expectBody('actor initial stream '+text,text);
    if(originProbe)report.retainedOriginProbe.push({label:'stream '+text,...await focus({action:'retained-range',mode:'read-origin'})});
   }
   segments[0].text=firstText;segments[0].revision++;
  }
  await command({action:'asr',segments});await expectBody('actor first input',firstText);
  if(originProbe)report.retainedOriginProbe.push({label:'first input',...await focus({action:'retained-range',mode:'read-origin'})});
  if(process.env.SAYWEAVE_ACTOR_LEARNING_SLOW==='1'){
   if(!readerStallAt||diagnostics.includes('owned_learning_read_stall=finished'))throw Error('Slow-reader overlap not established');
   report.inputContinuedWhileReaderBlocked=true;
   report.blockedReaderInputMs=Date.now()-readerStallAt;
  }
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
  if(caretOnly){
   if(sentinels||earlyEdit||commitEarly||backspace||firstOnly)throw Error('Caret-only primary requires the unchanged initial whole block');
   const logStart=diagnostics.length;
   const mouse=process.env.SAYWEAVE_BLOCK_CARET_MOUSE==='1';
   if(mouse){
    // Use the owned native fixture's physical pointer path. CDP mouse events
    // bypass the TSF/Win32 handoff and are not a product-input counterexample.
    const point=await page.evaluate(()=>{
     const editor=document.querySelector('#prosemirror'),walker=document.createTreeWalker(editor,NodeFilter.SHOW_TEXT);
     const text=walker.nextNode();if(!text||text.textContent!=='周三开会')throw Error('Original first text node unavailable');
     const range=document.createRange();range.setStart(text,2);range.setEnd(text,3);
     const rect=range.getBoundingClientRect();return {x:rect.left+1,y:rect.top+rect.height/2,innerWidth,innerHeight,dpr:devicePixelRatio};
    });
    const window=await focus({action:'read'});
    if(window.foregroundProcess!==pid||!window.foregroundClient)throw Error('Original owned pointer window changed');
    // Playwright's CSS device scale is independent of the Windows monitor DPI.
    const scale=window.foregroundClient[0]/point.innerWidth;
    const x=Math.round(point.x*scale);
    const y=Math.round(window.foregroundClient[1]-point.innerHeight*scale+point.y*scale);
    const click=await focus({action:'click-window',process:pid,window:window.foregroundWindow,x,y});
    report.caretPointer={point,client:window.foregroundClient,x,y,click};
    if(click.clicked!==2)throw Error('Guarded physical caret click not delivered');
    const observedUntil=Date.now()+1000;
    while(!diagnostics.slice(logStart).includes('operation=owned_pointer_probe')){
     if(Date.now()>=observedUntil)throw Error('Owned click did not traverse the production pointer hook before dispatch');
     await command({action:'status'});await settled();
    }
   }else{
    // Genuine navigation keys, without a text edit or a synthetic edited command.
    for(const key of ['End','ArrowLeft','ArrowLeft'])for(const down of [true,false]){
     const reply=await focus({action:'fixture-key',process:pid,key,down});
     if(!reply.keyResult?.accepted)throw Error('Caret navigation key refused');
    }
   }
   const position=await settled();
   report.caretOnly={before:firstText,mouse,position:position.dom.semanticSelection};
   if(position.text!==firstText||position.dom.semanticSelection.start!==2||position.dom.semanticSelection.end!==2)throw Error('Caret-only stimulus changed text or did not reach the middle');
   let ready;const until=Date.now()+5000;
   do{
    ready=await command({action:'status'});
    if(ready.audioRoute===1&&ready.published==='recording')break;
    await settled();
   }while(Date.now()<until);
   if(ready.audioRoute!==1||ready.published!=='recording'){
    report.firstDivergence={label:'caret-only resume without text edit',expected:'recording/audioRoute=1',actual:ready,body:(await settled()).text,sourceSelection:diagnostics.slice(logStart).includes('reason=sourceSelection')};
    throw Error('Caret-only movement never resumed dictation');
   }
   if(ready.requests!==0||(await settled()).text!==firstText)throw Error('Caret-only positioning generated or rewrote text');
   segments[0].confirmed=true;
   segments.push({id:2,revision:1,text:'下午三点',confirmed:false});
   await command({action:'asr',segments});await expectBody('caret-only middle literal','周三下午三点开会');
   segments[1].confirmed=true;
   await command({action:'asr',segments});await expectBody('caret-only whole model','周三下午三点开会。');
   const modeled=await settled();
   if(modeled.dom.semanticSelection.start!==modeled.text.length||modeled.dom.semanticSelection.end!==modeled.text.length)throw Error('Confirmed whole model did not leave caret at block end');
   segments.push({id:3,revision:1,text:'地点在上海',confirmed:false});
   await command({action:'asr',segments});await expectBody('caret-only next speech at end','周三下午三点开会。地点在上海');
   segments[2].confirmed=true;
   await command({action:'asr',segments});await expectBody('caret-only next whole model','周三下午三点开会，地点在上海。');
   if(diagnostics.slice(logStart).includes('operation=draft_block_sealed'))throw Error('Caret continuation silently replaced the original block');
   report.actorDelivery=await command({action:'finish'});
   if(!report.actorDelivery.finished||report.actorDelivery.error||report.actorDelivery.resultOnly)throw Error('Caret continuation did not finish direct input');
   await expectBody('caret-only final body',lastBody);
   if(process.env.SAYWEAVE_ACTOR_LEARNING==='1')await command({action:'done'});
   actor.stdin.end();
   if(!exited)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Caret actor cleanup timeout')),5000);actor.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Caret actor exit '+code));});});
   report.actorPassed=true;report.caretOnly.passed=true;return;
  }
  if(commitEarly){report.firstWriteBeforeNativeReceipt=!diagnostics.includes('revision=1 action=Update applied=true');if(!report.firstWriteBeforeNativeReceipt)throw Error('Initial commit barrier did not hold the native operation receipt');}
  if(earlyEdit&&report.actorTrackingReceiptBeforeEdit)throw Error('Early-edit timing not exercised: tracking receipt already consumed');
  if(retainedProbe){
   if(!physicalDate)throw Error('Retained range comparison requires physical date input');
   const sample=await focus({action:'retained-range',mode:'capture',process:pid,expected:firstText});
   (report.retainedRangeProbe??=[]).push({label:'before first key',...sample});
   if(sample.error)throw Error('Retained range capture unavailable: '+sample.error);
   retainedProbeActive=true;
  }
  for(const [index,replacement,tail,final]of (firstOnly?[]:[[0,backspace?'周三开会':'周五开会','，不对，改成下周一上午开会','下周一上午开会'],[1,'下周一开会','，改成下午三点开会','下周一下午三点开会'],[2,'下周一下午两点开会','，不对，改成线上四点开会','下周一下午四点线上开会']])){
   const before=await read();if(before.text!==lastBody)throw Error('Late actor body change before edit');
   const expectedEdit=replacement+(index===0?'':retained);
   let left=0,right=0;while(left<Math.min(before.text.length,expectedEdit.length)&&before.text[left]===expectedEdit[left])left++;while(right<Math.min(before.text.length,expectedEdit.length)-left&&before.text.at(-1-right)===expectedEdit.at(-1-right))right++;
   const eventWatermark=await page.evaluate(()=>window.ownedBlockEvents.length);
   const domWasComposing=await page.evaluate(()=>window.fixtureEditor.inspect().composing);
   if(!(index===0&&(backspace||physicalDate)))await command({action:'edited'});
   // End the native composition through the owned host's actual key route.
   // Mutating ProseMirror while TSF still owns a stale composition bypasses
   // normal editing and can itself manufacture an old-text commit on handoff.
   for(const down of [true,false]){
    const key=backspace&&index===0?'Backspace':handoffBarrier&&index===0?'End':'ArrowLeft';
    if(physicalDate)await physicalDateKey(key,down);
    else await focus({action:'fixture-key',process:pid,key,down});
   }
   if(backspace&&index===0)await command({action:'edited'});
   if(commitEarly&&index===0)writeFileSync(process.env.SAYWEAVE_INITIAL_BARRIER,'continue');
   if(domWasComposing)await page.waitForFunction(at=>window.ownedBlockEvents.slice(at).some(e=>e.type==='compositionend'),eventWatermark,{timeout:5000});
   const handedOff=await settled();trace.push({label:'actor edit handoff '+index,settled:handedOff});
   if(retainedProbeActive&&index===0)await readRetainedProbe('after first key and native handoff');
   if(originProbe&&index===0)report.retainedOriginProbe.push({label:'after handoff',...await focus({action:'retained-range',mode:'read-origin'})});
   if(handedOff.text!==(backspace&&index===0?replacement:before.text))throw Error('Native editing handoff changed body');
   if(handoffBarrier&&index===0){
    const deadline=Date.now()+5000;
    while(!diagnostics.includes('owned_handoff_barrier=waiting')){
     if(Date.now()>deadline)throw Error('Handoff receipt barrier was not reached');
     await settled();
    }
    report.handoffBarrier={stage:'EndComposition returned; next snapshot scheduling held',body:handedOff.text};
   }
   if(process.env.SAYWEAVE_BLOCK_SELECTION_PAUSE==='1'&&!(backspace&&index===0)){
    // Real editing starts by moving the caret/selection, before changing text.
    // A caret-only handoff must finish its real native read and resume even
    // before any text edit. It must not wait for the former sourceSelection
    // refusal, which is now reserved for missing source evidence.
    const logStart=diagnostics.length,deadline=Date.now()+5000;
    await command({action:'edited'});
    await page.evaluate(at=>window.fixtureEditor.select(at),prefix.length+left);
    for(;;){
     if(diagnostics.slice(logStart).includes('code=blockReleased')){
      report.firstDivergence={label:'selection before edit '+index,kind:'prematureBlockRelease',actual:(await read()).text};
      throw Error('Caret-only positioning retired the original block before manual edit');
     }
     const state=await command({action:'status'});
     if(diagnostics.slice(logStart).includes('action=Inspect applied=true code=blockReady')&&state.audioRoute===1&&state.published==='recording')break;
     if(Date.now()>=deadline)throw Error('Caret-only read did not resume recording');
     await settled();
    }
    const positioned=await settled();trace.push({label:'actor selection before edit '+index,settled:positioned});
    if(positioned.text!==before.text)throw Error('Caret-only positioning changed body');
    // The subsequent keyboard edit is a new interaction after that resume.
    await command({action:'edited'});
   }
   if(!(backspace&&index===0)){
    const edit={left:prefix.length+left,end:prefix.length+before.text.length-right,text:expectedEdit.slice(left,expectedEdit.length-right),caret:prefix.length+(interiorEdit&&index===0?expectedEdit.length-right:replacement.length)};
    if(keyboardEdit){
     let physicalSelection;
     if(physicalDate&&index===0){
      if(handoffBarrier){
       // Same key order as the actual Codex counterexample. A selection
       // modifier could itself synchronize the provider cache beforehand.
       for(let count=0;count<2;count++)for(const down of [true,false])await physicalDateKey('ArrowLeft',down);
       for(const down of [true,false])await physicalDateKey('Backspace',down);
       await page.waitForFunction(()=>window.fixtureEditor.read()==='周开会',undefined,{timeout:5000});
       physicalSelection=(await settled()).dom.semanticSelection;
      }else{
      // The first ArrowLeft above is the real handoff trigger. Select the
      // date with more real keys, not a DOM selection or an earlier edited
      // command which would change the operation under investigation.
      for(const down of [true,false])await physicalDateKey('ArrowLeft',down);
      await physicalDateKey('ShiftLeft',true);
      for(const down of [true,false])await physicalDateKey('ArrowLeft',down);
      await physicalDateKey('ShiftLeft',false);
      physicalSelection=(await settled()).dom.semanticSelection;
      if(physicalSelection.start!==1||physicalSelection.end!==2)throw Error('Physical date selection differs');
      }
     }else await page.evaluate(({left,end})=>window.fixtureEditor.select(left,end),edit);
     if(edit.text){const typed=await focus({action:'fixture-text',process:pid,text:edit.text});if(!typed.keyResult?.accepted)throw Error('Owned keyboard correction refused');}
     else for(const down of [true,false]){const key=await focus({action:'fixture-key',process:pid,key:'Backspace',down});if(!key.keyResult?.accepted)throw Error('Owned keyboard deletion refused');}
     await page.waitForFunction(text=>window.fixtureEditor.read()===text,prefix+expectedEdit+suffix,{timeout:6000});
     if(physicalDate&&index===0){
      for(const down of [true,false])await physicalDateKey('End',down);
      await command({action:'edited'});
      report.physicalDateCorrection={before:before.text,after:expectedEdit,selection:physicalSelection,method:handoffBarrier?'End/Left/Left/Backspace/UnicodeText/End':'ArrowLeft/Shift+ArrowLeft/UnicodeText/End'};
     }else await page.evaluate(caret=>window.fixtureEditor.select(caret),edit.caret);
    }else await page.evaluate(({left,end,text,caret})=>{window.fixtureEditor.edit(left,end,text);window.fixtureEditor.select(caret);},edit);
   }
   // Use this editor's supported paragraph break. A raw LF inserted into a
   // text node is normalized to a space by its own DOM parser, even during
   // ordinary typing without TSF. The separate host-control evidence retains
   // that failure; it is not a user paragraph-break acceptance case.
   if(userNewline&&index===0)await page.evaluate(at=>{window.fixtureEditor.appendParagraph();window.fixtureEditor.select(at);},prefix.length+replacement.length);
   const edited=await settled();trace.push({label:'actor manual edit '+index,settled:edited});if(edited.text!==replacement+retained)throw Error('Actor manual edit differs');
   if(handoffBarrier&&index===0){
    report.handoffBarrier.edited=edited.text;
    report.handoffBarrier.receiptStillHeld=!diagnostics.includes('owned_handoff_barrier=released');
    if(!report.handoffBarrier.receiptStillHeld)throw Error('Handoff barrier released before real keyboard edit');
    await command({action:'release-bind'});
   }
   if(retainedProbeActive&&index===0)await readRetainedProbe('after keyboard date correction');
   if(retainedProbeActive&&index===0&&process.env.SAYWEAVE_BLOCK_RETAINED_NEGATIVE){
    const mode=process.env.SAYWEAVE_BLOCK_RETAINED_NEGATIVE;
    if(mode==='clear-retype'){
     await page.evaluate(()=>window.fixtureEditor.edit(0,window.fixtureEditor.read().length,''));await settled();await readRetainedProbe('after clearing original field');
     await page.evaluate(text=>window.fixtureEditor.edit(0,0,text),replacement);await settled();await readRetainedProbe('after identical text retyped');
    }else if(mode==='rebuild'){
     await page.evaluate(()=>window.fixtureEditor.rebuildParagraph());await settled();await readRetainedProbe('after same-field paragraph rebuild');
    }else if(mode==='replace-children'){
     await page.evaluate(()=>{const paragraph=document.querySelector('#prosemirror .ProseMirror p');paragraph.replaceChildren(document.createTextNode(paragraph.textContent));});await settled();await readRetainedProbe('after same-paragraph child replacement');
    }else if(mode==='coalesced-clear'){
     await page.evaluate(()=>{const text=window.fixtureEditor.read();window.fixtureEditor.edit(0,text.length,'');window.fixtureEditor.edit(0,0,text);});await settled();await readRetainedProbe('after coalesced clear and same-text insertion');
    }else if(mode==='outside-prefix'){
     await page.evaluate(()=>window.fixtureEditor.edit(0,0,'块外|'));await settled();await readRetainedProbe('after outside prefix insertion');
    }else throw Error('Unknown retained-range negative case');
    if(process.env.SAYWEAVE_BLOCK_RETAINED_RESTORE_SELECTION==='1'){
     await page.evaluate(at=>window.fixtureEditor.select(at),replacement.length);await settled();await readRetainedProbe('after restoring original selection');
    }
    report.retainedRangeComparison=mode;
    if(process.env.SAYWEAVE_BLOCK_RETAINED_CONTINUE!=='1')throw Error('Readonly retained-range diagnostic completed: '+mode);
   }
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
   const logStart=diagnostics.length,deadline=Date.now()+5000;
   await command({action:'edited'});
   await page.evaluate(at=>window.fixtureEditor.select(at),prefix.length+1);
   for(;;){
    if(diagnostics.slice(logStart).includes('code=blockReleased'))throw Error('Final caret move discarded the block');
    const state=await command({action:'status'});
    if(diagnostics.slice(logStart).includes('action=Inspect applied=true code=blockReady')&&state.audioRoute===1&&state.published==='recording')break;
    if(Date.now()>=deadline)throw Error('Final caret-only read did not resume recording');
    await settled();
   }
  }
  const finishStarted=Date.now();
  report.actorDelivery=await command({action:'finish'});
  report.actorFinishMs=Date.now()-finishStarted;
  if(process.env.SAYWEAVE_BLOCK_FINISH_SELECTION==='1'&&report.actorFinishMs>3000)throw Error('Caret-only finish waited for another edit');
  if(!report.actorDelivery.finished||report.actorDelivery.error||report.actorDelivery.requests!==(firstOnly?1:3))throw Error('Actor final settlement or request count differs');
  await expectBody('actor final settled',firstOnly?'下周一上午开会':lastBody);
  if(process.env.SAYWEAVE_ACTOR_LEARNING==='1')await command({action:'done'});
  actor.stdin.end();
  if(!exited)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Actor cleanup timeout')),5000);actor.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Actor failed '+code+': '+diagnostics));});});
  report.actorPassed=true;
  })();
 }catch(error){actorFailure=error;throw error;}finally{
  const samplingErrors=[];
  if(rawCleanup)try{await rawCleanup();}catch(error){samplingErrors.push('Original stimulus retirement: '+String(error));}
  if(process.env.SAYWEAVE_BLOCK_HANDOFF_BARRIER==='1'&&!exited){
   try{await command({action:'release-bind'});}catch(error){samplingErrors.push('Handoff barrier release: '+String(error));}
  }
  if(diagnosticModifierHeld)try{
   const released=await focus({action:'fixture-key',process:pid,key:diagnosticKey,down:false});
   if(!released.keyResult?.accepted)samplingErrors.push('Owned diagnostic modifier release refused');
   else diagnosticModifierHeld=false;
  }catch(error){samplingErrors.push(String(error));}
  if(physicalShiftHeld)try{await physicalDateKey('ShiftLeft',false);}catch(error){samplingErrors.push(String(error));}
  if(retainedProbeActive){
   try{await readRetainedProbe('before actor cleanup');}catch(error){samplingErrors.push(String(error));}
   try{const released=await focus({action:'retained-range',mode:'release'});if(!released.released)samplingErrors.push('Retained range release failed');else retainedProbeActive=false;}catch(error){samplingErrors.push(String(error));}
  }
  try{report.actorBodyBeforeCleanup=await settled();}catch(error){samplingErrors.push(String(error));}
  try{
   if(!exited){actor.stdin.end();await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Actor retirement timeout')),5500);actor.once('exit',code=>{clearTimeout(timer);if(code===0)resolve();else reject(Error('Actor retirement exited '+code));});});}
   if(actorExit!==0)samplingErrors.push('Actor retirement exit was not successful: '+actorExit);
  }catch(error){samplingErrors.push(String(error));
  }finally{
   report.actorDiagnostics=diagnostics;
  }
  try{report.actorBodyAfterCleanup=await settled();}catch(error){samplingErrors.push(String(error));}
  if(!actorFailure&&report.actorBodyAfterCleanup?.text!==lastBody)samplingErrors.push('Actor retirement changed the last verified complete body');
  if(samplingErrors.length){
   report.actorCleanupSamplingErrors=samplingErrors;report.actorPassed=false;
   cleanupFailure=Error('Actor retirement/body verification failed: '+samplingErrors.join('; '));
  }
 }
 if(cleanupFailure)throw cleanupFailure;
 if(restartBody!==undefined){report.correctionRestartPassed=true;report.actorPassed=true;}
 return actorResult;
}
