// Scenario extension of test-prefix-input; all text belongs to its owned fixture.
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {writeFileSync} from 'node:fs';
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
 const step=async(label,action,text='',block)=>{
  const before=await read();if(lastBody!==undefined&&before.text!==lastBody)throw Error('Late host body change before '+label);
  child.stdin.write(JSON.stringify({action,text,block,turn,revision})+'\n');
  const native=await next();const immediate=await read();const afterEvents=await settled();
  const row={label,before,native,immediate,settled:afterEvents};trace.push(row);lastBody=afterEvents.text;
  if(!native.reply.applied){report.firstDivergence={label,kind:'nativeRefusal',code:native.reply.code};throw Error(label+': '+native.reply.code);}
  if(native.reply.block)snapshot=native.reply.block;
  return row;
 };
 const command=action=>({lease:snapshot.lease,id:snapshot.id,action,epoch:snapshot.epoch,version:snapshot.version,expected:snapshot.text,selectionStart:snapshot.selectionStart,selectionEnd:snapshot.selectionEnd});
 const check=(row,expected)=>{if(row.settled.text!==expected){report.firstDivergence={label:row.label,kind:'body',expected,actual:row.settled.text};throw Error(row.label+': actual rich-text body differs');}};
 try{
  report.executionIdentity=await next();
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
  if(!ended)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Native probe did not retire')),5000);child.once('exit',code=>{clearTimeout(timer);code===0?resolve():reject(Error('Native probe cleanup '+code));});});
  await runActor({page,pid,start,read,settled,trace,report});
 }finally{
  report.blockEvents=await page.evaluate(()=>window.ownedBlockEvents);
  writeFileSync(path.join(folder,'block-trace.json'),JSON.stringify({trace,events:report.blockEvents,identity:report.executionIdentity,firstDivergence:report.firstDivergence,errors},null,2));
  if(!ended)child.stdin.end('exit\n');
 }
}

async function runActor({page,pid,start,read,settled,trace,report}){
 const executable=process.env.SAYWEAVE_ACTOR_TEST_EXE;
 if(!executable||!path.isAbsolute(executable))throw Error('Built owned actor test executable required');
 await page.evaluate(()=>window.fixtureEditor.set('',0,0));
 await settled();
 const actor=start(executable,['--exact','dictation::drafts::tests::owned::richtext_actor','--ignored','--nocapture','--test-threads=1'],{SAYWEAVE_INPUT_ACCEPTANCE:'1',SAYWEAVE_TSF_OWNED_PID:String(pid),SAYWEAVE_DRAFT_STRATEGY:'revisable'});
 let buffer='',diagnostics='',exited=false;const values=[],pending=[];
 actor.stderr.on('data',data=>diagnostics+=data.toString('utf8'));
 actor.stdout.on('data',data=>{buffer+=data.toString('utf8');let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at);buffer=buffer.slice(at+1);const marker=line.indexOf('ACTOR_JSON:');if(marker<0){diagnostics+=line+'\n';continue;}const value=JSON.parse(line.slice(marker+11));if(pending.length)pending.shift().resolve(value);else values.push(value);}});
 actor.once('exit',code=>{exited=true;for(const item of pending.splice(0))item.reject(Error('Actor exited '+code+': '+diagnostics));});
 const receive=()=>values.length?Promise.resolve(values.shift()):new Promise((resolve,reject)=>{if(exited){reject(Error(diagnostics));return;}const item={resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}};const timer=setTimeout(()=>{const index=pending.indexOf(item);if(index>=0)pending.splice(index,1);reject(Error('Actor response timeout: '+diagnostics));},18000);pending.push(item);});
 const command=async value=>{actor.stdin.write(JSON.stringify(value)+'\n');const reply=await receive();trace.push({label:'actor '+value.action,reply});return reply;};
 let lastBody='';const segments=[];
 const expectBody=async(label,expected)=>{
  const deadline=Date.now()+5000;let sample;
  do{sample=await settled();trace.push({label,settled:sample});if(sample.text===expected){lastBody=expected;return;}await command({action:'status'});}while(Date.now()<deadline);
  report.firstDivergence={label,kind:'actorBody',expected,actual:sample.text};throw Error(label+': coordinator body differs');
 };
 try{
  report.actorIdentity=await receive();
  segments.push({id:1,revision:1,text:'周三开会',confirmed:false});
  await command({action:'asr',segments});await expectBody('actor first input','周三开会');
  for(const [index,replacement,tail,final]of [[0,'周五开会','，不对，改成下周一上午开会','下周一上午开会'],[1,'下周一开会','，改成下午三点开会','下周一下午三点开会'],[2,'下周一下午两点开会','，不对，改成线上四点开会','下周一下午四点线上开会']]){
   const before=await read();if(before.text!==lastBody)throw Error('Late actor body change before edit');
   let left=0,right=0;while(left<Math.min(before.text.length,replacement.length)&&before.text[left]===replacement[left])left++;while(right<Math.min(before.text.length,replacement.length)-left&&before.text.at(-1-right)===replacement.at(-1-right))right++;
   await command({action:'edited'});
   await page.evaluate(({left,end,text,caret})=>{window.fixtureEditor.edit(left,end,text);window.fixtureEditor.select(caret);},{left,end:before.text.length-right,text:replacement.slice(left,replacement.length-right),caret:replacement.length});
   const edited=await settled();trace.push({label:'actor manual edit '+index,settled:edited});if(edited.text!==replacement)throw Error('Actor manual edit differs');
   segments.forEach(s=>s.confirmed=true);
   segments.push({id:index+2,revision:1,text:tail.slice(0,3),confirmed:false});
   await command({action:'asr',segments});await expectBody('actor partial '+index,replacement+tail.slice(0,3));
   segments.at(-1).revision++;segments.at(-1).text=tail;segments.at(-1).confirmed=true;
   await command({action:'asr',segments});await expectBody('actor model '+index,final);
  }
  report.actorDelivery=await command({action:'finish'});
  if(!report.actorDelivery.finished||report.actorDelivery.error||report.actorDelivery.requests!==3)throw Error('Actor final settlement or request count differs');
  await expectBody('actor final settled',lastBody);
  actor.stdin.end();
  if(!exited)await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Actor cleanup timeout')),5000);actor.once('exit',code=>{clearTimeout(timer);code===0?resolve():reject(Error('Actor failed '+code+': '+diagnostics));});});
  report.actorPassed=true;
 }finally{
  if(!exited){actor.stdin.end();await new Promise(resolve=>{const timer=setTimeout(resolve,5500);actor.once('exit',()=>{clearTimeout(timer);resolve();});});}
  report.actorDiagnostics=diagnostics;
 }
}
