// Scenario extension of test-prefix-input; all text belongs to its owned fixture.
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {writeFileSync} from 'node:fs';

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
  await expectBody('reacquired model result','下周一上午开会');
  if(!/code=draftOwnershipTerminated:[^\n]+mutation=NotAttempted/.test(diagnostics))throw Error('Exact no-write termination receipt not observed');
  const state=await command({action:'status'});
  if(state.audioRoute!==1||state.published!=='recording')throw Error('Recording projection did not resume');
  report.actorDelivery=await command({action:'finish'});
  if(!report.actorDelivery.finished||report.actorDelivery.error||report.actorDelivery.requests!==1)throw Error('Final settlement differs');
  await expectBody('after final host events','下周一上午开会');
  report.terminationRecovered=true;
 }finally{
  if(!ended){actor.stdin.end();await new Promise(resolve=>{const timer=setTimeout(resolve,5500);actor.once('exit',()=>{clearTimeout(timer);resolve();});});}
  report.actorDiagnostics=diagnostics;
 }
}

export async function runBlockRegression({page,pid,focus,start,root,folder,report}){
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
 const settled=()=>page.evaluate(async()=>{
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
  const earlyEdit=process.env.SAYWEAVE_BLOCK_EARLY_EDIT==='1';
  report.actorScenario=earlyEdit?'edit-before-tracking-receipt':'edit-after-tracking-receipt';
  const backspace=process.env.SAYWEAVE_BLOCK_BACKSPACE==='1';
  const userNewline=process.env.SAYWEAVE_BLOCK_USER_NEWLINE==='1';
  if(userNewline&&!backspace)throw Error('User newline regression requires direct Backspace setup');
  if(userNewline)report.actorNewlineRepresentation='paragraph';
  const retained=userNewline?'\n':'';
  const firstText=backspace?'周三开会。':'周三开会';
  segments.push({id:1,revision:1,text:firstText,confirmed:false});
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
  while(!earlyEdit&&!diagnostics.includes('operation=draft_block_bound')){
   if(Date.now()>=bindDeadline)throw Error('Production initial block did not bind');
   await command({action:'status'});await settled();
  }
  report.actorTrackingReceiptBeforeEdit=diagnostics.includes('operation=draft_block_bound');
  if(earlyEdit&&report.actorTrackingReceiptBeforeEdit)throw Error('Early-edit timing not exercised: tracking receipt already consumed');
  for(const [index,replacement,tail,final]of [[0,backspace?'周三开会':'周五开会','，不对，改成下周一上午开会','下周一上午开会'],[1,'下周一开会','，改成下午三点开会','下周一下午三点开会'],[2,'下周一下午两点开会','，不对，改成线上四点开会','下周一下午四点线上开会']]){
   const before=await read();if(before.text!==lastBody)throw Error('Late actor body change before edit');
   const expectedEdit=replacement+(index===0?'':retained);
   let left=0,right=0;while(left<Math.min(before.text.length,expectedEdit.length)&&before.text[left]===expectedEdit[left])left++;while(right<Math.min(before.text.length,expectedEdit.length)-left&&before.text.at(-1-right)===expectedEdit.at(-1-right))right++;
   const eventWatermark=await page.evaluate(()=>window.ownedBlockEvents.length);
   if(!(backspace&&index===0))await command({action:'edited'});
   // End the native composition through the owned host's actual key route.
   // Mutating ProseMirror while TSF still owns a stale composition bypasses
   // normal editing and can itself manufacture an old-text commit on handoff.
   for(const down of [true,false])await focus({action:'fixture-key',process:pid,key:backspace&&index===0?'Backspace':'ArrowLeft',down});
   if(backspace&&index===0)await command({action:'edited'});
   await page.waitForFunction(at=>window.ownedBlockEvents.slice(at).some(e=>e.type==='compositionend'),eventWatermark,{timeout:5000});
   const handedOff=await settled();trace.push({label:'actor edit handoff '+index,settled:handedOff});
   if(handedOff.text!==(backspace&&index===0?replacement:before.text))throw Error('Native editing handoff changed body');
   if(process.env.SAYWEAVE_BLOCK_SELECTION_PAUSE==='1'){
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
   if(!(backspace&&index===0))await page.evaluate(({left,end,text,caret})=>{window.fixtureEditor.edit(left,end,text);window.fixtureEditor.select(caret);},{left:prefix.length+left,end:prefix.length+before.text.length-right,text:expectedEdit.slice(left,expectedEdit.length-right),caret:prefix.length+replacement.length});
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
   await command({action:'asr',segments});await expectBody('actor partial '+index,replacement+tail.slice(0,3)+retained);
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
  if(!report.actorDelivery.finished||report.actorDelivery.error||report.actorDelivery.requests!==3)throw Error('Actor final settlement or request count differs');
  await expectBody('actor final settled',lastBody);
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
