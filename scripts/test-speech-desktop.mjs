// Actual WebView, Windows vault, microphone and disk; ASR text comes from an isolated TLS fixture.
// No production trust override and no supplier request. Reports stay under .local.
import {chromium,expect} from '@playwright/test';
import {build as buildFixture} from 'vite';
import {spawn,execFileSync} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
import {createServer} from 'node:https';
import {gzipSync,gunzipSync} from 'node:zlib';
import {createWriteStream,mkdirSync,readFileSync,writeFileSync,unlinkSync,existsSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {beginValidation,finishValidation} from './storage-policy.mjs';
import {verifyComponentEmbeddings} from './build-text-service.mjs';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),id=randomUUID();
const storageContext=beginValidation(root,{id,kind:'speech-desktop'});
const directory=path.join(root,'.local','data-acceptance',id),data=path.join(directory,'data');mkdirSync(data,{recursive:true});
const exe=path.join(root,'src-tauri','target','release','desktop-acceptance.exe'),port=9241,key='fixture-'+randomUUID();
const polishCase=process.argv.find(value=>value.startsWith('--polish-case='))?.slice('--polish-case='.length);
if(polishCase&&!['cancel','no-history'].includes(polishCase))throw Error('Unknown polish acceptance case');
const polishOnly=process.argv.includes('--polish')||Boolean(polishCase);
const report={id,passed:false,checks:[],connections:[],handshakes:[],polishRequests:[]},delay=ms=>new Promise(r=>setTimeout(r,ms));
report.controllerSha256=createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex');
let app,browser,server,mode='normal',expectedProtocol='qwen',expectedModel='qwen-audio-3.1-asr-flash-streaming';const sockets=new Set(),polishReleases=new Map(),polishChannels=[],copyReleases=new Map();
const restartChannels=[],readinessChannels=[],translationRequests=[],translationReleases=new Map();
async function command(args,extraEnv={}){
 const fixture=args[0]==='test'&&args.includes('--lib')?args.find(value=>['model_desktop_fixture','model_desktop_cleanup'].includes(value)):null;
 const retained=fixture&&report.reusedBuild?.fixtureTest;
 const child=spawn(retained??'cargo',retained?['--exact','models::tests::'+fixture,'--ignored','--test-threads=1']:args,{cwd:root,windowsHide:true,env:{...process.env,...extraEnv,SAYWEAVE_DATA_ACCEPTANCE_ID:id},stdio:['ignore','pipe','pipe']});
 const log=createWriteStream(path.join(directory,'fixture.log'),{flags:'a'});child.stdout.pipe(log);child.stderr.pipe(log);if(await new Promise((r,j)=>{child.on('exit',r);child.on('error',j);})!==0)throw Error('Fixture operation failed');
}
function frame(value,opcode=1){const payload=Buffer.isBuffer(value)?value:Buffer.from(JSON.stringify(value));const header=Buffer.alloc(payload.length<126?2:4);header[0]=0x80|opcode;header[1]=payload.length<126?payload.length:126;if(payload.length>=126)header.writeUInt16BE(payload.length,2);return Buffer.concat([header,payload]);}
function websocket(req,socket,head){
 const doubao=expectedProtocol==='doubao';
 report.handshakes.push({protocol:expectedProtocol,mode,path:req.url,resource:req.headers['x-api-resource-id']??null});
 if(mode==='auth'||req.url!==(doubao?'/api/v3/sauc/bigmodel_async':'/api-ws/v1/inference')||(doubao?req.headers['x-api-key']!==key:req.headers.authorization!=='Bearer '+key)){socket.end('HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n');return;}
 if(doubao&&mode==='resource-denied'&&req.headers['x-api-resource-id']==='volc.seedasr.sauc.duration'){socket.end('HTTP/1.1 403 Forbidden\r\nX-Api-Status-Code: 45000030\r\nContent-Length: 0\r\n\r\n');return;}
 if(doubao){expect(req.headers['x-api-resource-id']).toBe(mode==='resource-denied'?'volc.seedasr.sauc.concurrent':'volc.seedasr.sauc.duration');expect(req.headers['x-api-request-id']).toMatch(/^[a-f0-9-]{36}$/);expect(req.headers.authorization).toBeUndefined();}

 const accept=createHash('sha1').update(req.headers['sec-websocket-key']+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: '+accept+'\r\n\r\n');sockets.add(socket);socket.on('close',()=>sockets.delete(socket));socket.on('error',()=>{});
 const connection={protocol:expectedProtocol,mode,model:null,frames:0,bytes:0,started:false,finished:false};report.connections.push(connection);let task,buffer=head,fragment=Buffer.alloc(0),fragmentOpcode=0;
 const send=(event,payload={})=>socket.write(frame({header:{task_id:task,event},payload}));
 const sentence=(sid,text,end)=>send('result-generated',{output:{sentence:{sentence_id:sid,sentence_end:end,text,begin_time:0,end_time:end?connection.bytes/32:null}}});
 if(connection.mode.startsWith('learning-restart-'))restartChannels.push({connection,sentence});
 const result=(text,end)=>sentence(1,text,end);
 if(connection.mode==='copy-only')copyReleases.set(connection,()=>{connection.released=true;result('正在识别',false);});
 const raceOrdinal=polishChannels.filter(c=>c.mode===connection.mode).length;
 if(connection.mode.startsWith('polish-'))polishChannels.push({mode:connection.mode,ordinal:raceOrdinal,sentence,finished:()=>send('task-finished')});
 const binaryPayload=(value,last=false)=>{const bytes=gzipSync(Buffer.from(JSON.stringify(value)));connection.responseSequence=(connection.responseSequence??0)+1;const header=Buffer.alloc(12);header.set([0x11,last?0x93:0x91,0x11,0]);header.writeInt32BE(last?-1:connection.responseSequence,4);header.writeUInt32BE(bytes.length,8);socket.write(frame(Buffer.concat([header,bytes]),2));};
 const binaryResult=(text,end,last=false)=>binaryPayload({code:20000000,result:{text,utterances:text?[{text,definite:end,start_time:0,end_time:Math.round(connection.bytes/32)}]:[]}},last);
 function consume(opcode,payload){
  if(doubao&&opcode===2){
   const kind=payload[1]>>4,flags=payload[1]&15;expect(payload.readUInt32BE(4)).toBe(payload.length-8);const body=gunzipSync(payload.subarray(8));
   if(kind===1){const value=JSON.parse(body.toString());expect(value.audio).toMatchObject({format:'pcm',rate:16000,bits:16,channel:1});expect(value.request).toMatchObject({model_name:'bigmodel',result_type:'full',show_utterances:true});connection.model='Doubao-Seed-ASR-2.0';connection.started=true;binaryResult('',false);return;}
   expect(kind).toBe(2);expect(connection.started).toBe(true);
   if(flags&2){connection.finished=true;if(connection.mode==='pending')return;if(connection.mode==='empty'||connection.mode==='empty-ack'){binaryResult('',true,true);return;}if(connection.mode==='malformed-terminal'){binaryPayload({code:20000000,result:{text:123}},true);return;}const complete=()=>{const text=connection.mode==='single-a'?'a':connection.mode==='unicode'?'中文，🙂\n第二行 e\u0301。':'原生受控识别完成。';binaryResult(text,true);binaryResult('旧暂定不能覆盖。',false);binaryResult(text,true,true);};if(connection.mode==='delayed-final')setTimeout(complete,650);else complete();return;}
   connection.frames++;connection.bytes+=body.length;if(connection.frames===1)binaryResult('正在',false);if(connection.frames===8)binaryResult('正在识别',false);return;
  }

  if(opcode===8){socket.end(frame(payload,8));return;}if(opcode===9){socket.write(frame(payload,10));return;}
  if(connection.mode.startsWith('learning-restart-')){
   const text=connection.mode.endsWith('-first')?'Seville':'地点在上海';
   if(opcode===2){
    if(!connection.started)throw Error('Restart audio arrived before task-started');
    connection.frames++;connection.bytes+=payload.length;
    if(connection.mode.endsWith('-first')){
     if(connection.frames===1)result(text,false);
     if(connection.frames===8)result(text,true);
    }
    return;
   }
   if(opcode===1&&JSON.parse(payload.toString()).header.action==='finish-task'){
    connection.finished=true;result(text,true);send('task-finished');return;
   }
  }
  if(opcode===2){if(!connection.started)throw Error('Audio arrived before task-started');connection.frames++;connection.bytes+=payload.length;if(connection.mode==='copy-only')return;if(connection.mode.startsWith('polish-')){const text=connection.mode==='polish-edit'?(raceOrdinal===0?'呃，编辑前啊。':'嗯，继续输入🙂。'):connection.mode==='polish-revision'?'嗯，修订前啊。':'呃，等待原文啊。';if(connection.frames===1)sentence(1,text,false);if(connection.frames===8)sentence(1,text,true);return;}if(connection.mode==='polish'){if(connection.frames===1)sentence(1,'呃，第一段啊。',false);if(connection.frames===8)sentence(1,'呃，第一段啊。',true);if(connection.frames===16)sentence(2,'嗯，第二段啊。',false);if(connection.frames===35)sentence(2,'嗯，第二段啊。',true);if(connection.frames===45)sentence(3,'尾部',false);return;}if(connection.frames===1){if(connection.mode==='failure'){socket.write(frame({header:{task_id:task,event:'task-failed',error_code:'SERVICE_ERROR',error_message:'PRIVATE FIXTURE ERROR'}}));return;}result('正在',false);}if(connection.frames===8)result('正在识别',false);return;}
  if(opcode!==1)return;const value=JSON.parse(payload.toString());if(value.header.action==='run-task'){task=value.header.task_id;expect(value.payload.parameters.sample_rate).toBe(16000);expect(value.payload.parameters.format).toBe('pcm');expect(value.payload.model).toBe(expectedModel);connection.model=value.payload.model;
   if(connection.mode.startsWith('readiness-')){
    const channel={connection,receivedAt:Date.now(),released:false,release(){if(this.released)return this.releaseDelivery;this.released=true;this.releasedAt=Date.now();const sent=!socket.destroyed&&socket.writable;this.releaseDelivery={at:this.releasedAt,sent,socketDestroyed:socket.destroyed,socketWritable:socket.writable};if(sent){connection.started=true;send('task-started');}return this.releaseDelivery;}};
    readinessChannels.push(channel);
   }else{connection.started=true;send('task-started');}}
  else if(value.header.action==='finish-task'){connection.finished=true;if(connection.mode.startsWith('polish-')){if(connection.mode==='polish-edit'&&raceOrdinal===0)sentence(2,'旧轮尾字不应覆盖。',true);send('task-finished');return;}if(connection.mode==='polish'){sentence(3,'尾部完成🙂',true);send('task-finished');return;}if(connection.mode==='pending')return;if(connection.mode==='copy-only'&&!connection.released){send('task-finished');return;}if(connection.mode==='empty'){send('task-finished');return;}result('原生受控识别完成。',true);result('重复最终结果不应覆盖。',true);send('task-finished');}
 }
 function receive(chunk){buffer=Buffer.concat([buffer,chunk]);try{while(buffer.length>=2){const opcode=buffer[0]&15,fin=(buffer[0]&128)!==0,masked=(buffer[1]&128)!==0;let length=buffer[1]&127,offset=2;if(length===126){if(buffer.length<4)return;length=buffer.readUInt16BE(2);offset=4;}else if(length===127){if(buffer.length<10)return;length=Number(buffer.readBigUInt64BE(2));offset=10;}if(length>1024*1024)throw Error('oversize frame');if(masked)offset+=4;if(buffer.length<offset+length)return;const payload=Buffer.from(buffer.subarray(offset,offset+length));if(masked){const mask=buffer.subarray(offset-4,offset);for(let n=0;n<length;n++)payload[n]^=mask[n%4];}buffer=buffer.subarray(offset+length);if(opcode===0){fragment=Buffer.concat([fragment,payload]);if(fin){consume(fragmentOpcode,fragment);fragment=Buffer.alloc(0);}}else if(!fin&&opcode<8){fragmentOpcode=opcode;fragment=payload;}else consume(opcode,payload);}}catch(e){report.protocolError=String(e);socket.destroy();}}
 socket.on('data',receive);if(head.length)receive(Buffer.alloc(0));
}
async function modelFixture(req,res){
 if(req.url!=='/v1/chat/completions'||req.headers.authorization!=='Bearer '+key){res.writeHead(401);res.end('{}');return;}
 let bytes=0;const chunks=[];for await(const chunk of req){bytes+=chunk.length;if(bytes>16*1024){res.writeHead(413);res.end();return;}chunks.push(chunk);}
 const request=JSON.parse(Buffer.concat(chunks).toString('utf8')),data=JSON.parse(request.messages[1].content);
 if(report.scope==='translation'&&(Array.isArray(data.observations)||typeof data.before==='string')){
  expect(request.model).toBe('translation-fixture');
  const profile=Array.isArray(data.observations);
  expect(request.messages[0].content).toContain(profile?'更新表达偏好':'真实修改');
  if(profile){expect(typeof data.scope).toBe('string');expect(Array.isArray(data.current)).toBe(true);const translated=new Set((report.translation?.rounds??[]).filter(r=>r.case!=='ordinary-after-translation').map(r=>r.taskId));for(const observation of [...data.observations,...(data.prior_support??[])])expect(translated.has(observation.session)).toBe(false);}
  else expect(typeof data.after).toBe('string');
  report.translationLearningRequests??=[];report.translationLearningRequests.push({kind:profile?'profile':'edit',mode,at:performance.now(),request:structuredClone(request)});
  const output=JSON.stringify(profile?{outcome:'unchanged',preferences:[],candidates:[],evidence:[],reason:'受控服务保留现有偏好'}:{class:'uncertain',original:null,term:null,summary:null});
  if(request.stream){res.writeHead(200,{'Content-Type':'text/event-stream'});res.end('data: '+JSON.stringify({choices:[{index:0,delta:{role:'assistant',content:output},finish_reason:null}]})+'\n\ndata: '+JSON.stringify({choices:[{index:0,delta:{},finish_reason:'stop'}]})+'\n\ndata: [DONE]\n\n');}
  else{res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({choices:[{index:0,message:{role:'assistant',content:output},finish_reason:'stop'}]}));}
  return;
 }
 if(report.scope==='translation'&&mode==='normal'){
  expect(request.stream).toBe(true);expect(request.model).toBe('translation-fixture');expect(typeof data.input_text).toBe('string');
  report.ordinaryTranslationRegressionRequests??=[];report.ordinaryTranslationRegressionRequests.push(structuredClone(request));
  res.writeHead(200,{'Content-Type':'text/event-stream'});
  res.end('data: '+JSON.stringify({choices:[{index:0,delta:{role:'assistant',content:data.input_text},finish_reason:null}]})+'\n\ndata: '+JSON.stringify({choices:[{index:0,delta:{},finish_reason:'stop'}]})+'\n\ndata: [DONE]\n\n');return;
 }
 if(mode.startsWith('translation-')){
  expect(request.stream).toBe(true);expect(request.model).toBe('translation-fixture');
  const receipt={mode,request:structuredClone(request),at:performance.now(),source:data.source_text};translationRequests.push(receipt);
  await new Promise(resolve=>{const timer=setTimeout(resolve,15000);const release=()=>{clearTimeout(timer);resolve();};translationReleases.set(receipt,release);res.once('close',release);});
  translationReleases.delete(receipt);if(res.destroyed){receipt.aborted=true;return;}
  if(receipt.mode==='translation-failure'){res.writeHead(500);res.end('{}');return;}
  res.writeHead(200,{'Content-Type':'text/event-stream'});
  res.end('data: '+JSON.stringify({choices:[{index:0,delta:{role:'assistant',content:'午後3時に会議です。'},finish_reason:null}]})+'\n\ndata: '+JSON.stringify({choices:[{index:0,delta:{},finish_reason:'stop'}]})+'\n\ndata: [DONE]\n\n');receipt.responded=true;return;
 }
 if(mode.startsWith('learning-restart-')){
  report.restartModelRequests??=[];
  report.restartModelRequests.push({mode,stream:request.stream,input:data.input_text??null,at:performance.now()});
  const output=request.stream?(mode.includes('-tab-')?data.input_text.replace('舟三','周三'):data.input_text):JSON.stringify({class:'lexical',original:'Seville',term:'sayweave',summary:null});
  if(request.stream){res.writeHead(200,{'Content-Type':'text/event-stream'});res.end('data: '+JSON.stringify({choices:[{index:0,delta:{role:'assistant',content:output},finish_reason:null}]})+'\n\ndata: '+JSON.stringify({choices:[{index:0,delta:{},finish_reason:'stop'}]})+'\n\ndata: [DONE]\n\n');}
  else{res.writeHead(200,{'Content-Type':'application/json'});res.end(JSON.stringify({choices:[{index:0,message:{role:'assistant',content:output},finish_reason:'stop'}]}));}
  return;
 }
 expect(request.stream).toBe(true);expect(request.model).toBe('polish-fixture');
 const receipt={input:data.input_text,context:data.previous_context,bytes,at:performance.now(),mode},requestKey=report.polishRequests.length+':'+data.input_text;report.polishRequests.push(receipt);
 const output=data.input_text.includes('第一段')?'第一段。\n':data.input_text.includes('第二段')?'第二段。\n':data.input_text.includes('继续输入')?'继续输入🙂。':data.input_text.includes('修订后')?'修订后🙂。':data.input_text;
 if(!data.input_text.includes('尾部完成')){
  await new Promise(resolve=>{const timer=setTimeout(resolve,30000);const release=()=>{clearTimeout(timer);resolve();};polishReleases.set(requestKey,release);res.once('close',()=>{receipt.closedAt=performance.now();release();});});
  polishReleases.delete(requestKey);if(res.destroyed){receipt.aborted=true;return;}
 }
 receipt.respondedAt=performance.now();
 res.writeHead(200,{'Content-Type':'text/event-stream'});
 const chunk=JSON.stringify({choices:[{index:0,delta:{role:'assistant',content:output},finish_reason:null}]});
 const wire=Buffer.from('data: '+chunk+'\r\n\r\ndata: '+JSON.stringify({choices:[{index:0,delta:{},finish_reason:'stop'}]})+'\n\ndata: [DONE]\n\n');
 for(let n=0;n<wire.length;n+=7){res.write(wire.subarray(n,n+7));}res.end();
}
// Same production coordinator, actual microphone and owned native target. Only supplier replies are controlled.
async function translationScenario(page,base){
 await invoke(page,'model_save',{request:{id:null,revision:null,expectation:null,input:{name:'隔离翻译文字',protocol:'openai-chat',baseUrl:base+'/v1',modelId:'translation-fixture',usages:['text']},key:{action:'replace',value:key}}});
 const keys=await invoke(page,'shortcut_status');await invoke(page,'shortcut_save',{config:{...keys.saved,binding:'AltRight',translationBinding:'AltRight+ShiftRight',mode:'toggle'}});
 const tapConfig=(await invoke(page,'shortcut_status')).saved;
 expect((await rejected(page,'shortcut_save',{config:{...tapConfig,mode:'hold'}})).code).toBe('shortcutHoldConflict');
 expect((await invoke(page,'shortcut_status')).saved).toEqual(tapConfig);
 const configuration=await invoke(page,'translation_settings');
 expect(configuration.settings.targets).toEqual(['en-US','ja','ko','fr','zh-Hans']);
 const editor=await ownedEditor(),overlay=await overlayPage();
 // Native focus loss is part of recording ownership; Playwright enables
 // synthetic always-focused pages by default, masking actual window blur.
 const captureCdp=await page.context().newCDPSession(page);
 await captureCdp.send('Emulation.setFocusEmulationEnabled',{enabled:false});
 try {
 await invoke(page,'shortcut_save',{config:{...(await invoke(page,'shortcut_status')).saved,translationBinding:'F8'}});
 await invoke(page,'runtime_window',{surface:'main',action:'show'});
 await editor.command({action:'focus-process',process:app.pid});
 await page.getByRole('button',{name:'设置',exact:true}).click();
 await page.getByRole('button',{name:'更改翻译快捷键',exact:true}).click();
 await expect.poll(async()=>(await invoke(page,'shortcut_status')).capturing).toBe(true);
 report.captureProbe={before:await invoke(page,'shortcut_status'),cases:[]};
 await page.evaluate(()=>{window.__captureEdges=[];for(const type of ['keydown','keyup'])window.addEventListener(type,e=>window.__captureEdges.push({type,code:e.code,trusted:e.isTrusted}),true);});
 const keyEvent=async(key,down)=>{const value=await editor.command({action:'fixture-key',process:app.pid,...(Array.isArray(key)?{keys:key}:{key}),down});expect(value.keyResult?.accepted).toBe(true);};
 const beginCapture=async(field='translationBinding')=>{
  await page.getByRole('button',{name:field==='binding'?'更改快捷键':'更改翻译快捷键',exact:true}).click();
  await expect.poll(async()=>(await invoke(page,'shortcut_status')).capturing).toBe(true);
  return (await invoke(page,'shortcut_status')).captureToken;
 };
 const finishKeys=async(events,expected,label,field='translationBinding')=>{
  const before=await invoke(page,'shortcut_status');
  for(const [key,down] of events)await keyEvent(key,down);
  await expect.poll(async()=>(await invoke(page,'shortcut_status')).capturing,{timeout:4000}).toBe(false);
  await expect.poll(async()=>(await invoke(page,'shortcut_status')).saved[field],{timeout:4000}).toBe(expected);
  const after=await invoke(page,'shortcut_status');expect(after.active[field]).toBe(expected);
  report.captureProbe.cases.push({label,events,before,after});
 };
 // F8 was already registered before entering the editor: its Down must now reach capture.
 await finishKeys([['F8',true],['F8',false]],'F8','registered F8');
 for(const reason of ['button','outside','escape','blur']){
  const before=await invoke(page,'shortcut_status'),token=await beginCapture();
  if(reason==='button')await page.getByRole('button',{name:'取消录入',exact:true}).click();
  if(reason==='outside')await page.getByText('口述与翻译',{exact:true}).click();
  if(reason==='escape'){await keyEvent('Escape',true);await keyEvent('Escape',false);}
  if(reason==='blur'){const focus=await editor.command({action:'focus',text:'',start:0,end:0});report.captureProbe.blurTransition={focus};expect(focus.foregroundProcess).toBe(editor.pid);await expect.poll(()=>page.evaluate(()=>document.hasFocus())).toBe(false);}
  await expect.poll(async()=>(await invoke(page,'shortcut_status')).capturing).toBe(false);
  const after=await invoke(page,'shortcut_status');expect(after.saved).toEqual(before.saved);expect(after.active).toEqual(before.active);
  report.captureProbe.cases.push({label:'cancel '+reason,token,before,after});
  if(reason==='blur'){await editor.command({action:'focus-process',process:app.pid});await expect.poll(()=>page.evaluate(()=>document.hasFocus())).toBe(true);}
 }
 const stale=await beginCapture();await page.getByRole('button',{name:'取消录入',exact:true}).click();
 await expect.poll(async()=>(await invoke(page,'shortcut_status')).capturing).toBe(false);
 const fresh=await beginCapture();
 expect(await invoke(page,'shortcut_capture_candidate',{token:stale,binding:'F9'})).toMatchObject({capturing:true,captureToken:fresh});
 expect(await invoke(page,'shortcut_capture',{begin:false,token:stale})).toMatchObject({capturing:true,captureToken:fresh});
 await finishKeys([['ControlLeft',true],['F8',true],['ControlLeft',false],['F8',false]],'Control+F8','Control+F8 releases modifier first');
 const stable=await invoke(page,'shortcut_status');
 expect((await invoke(page,'shortcut_capture_candidate',{token:fresh,binding:'F9'})).saved).toEqual(stable.saved);
 await beginCapture();
 await finishKeys([['ControlLeft',true],['F8',true],['F8',false],['ControlLeft',false]],'Control+F8','registered chord re-recorded');
 for(const [events,message,label] of [
  [[['F12',true],['F12',false]],'此快捷键已保留供系统使用。','reserved F12'],
  [[['AltLeft',true],['AltLeft',false]],'此快捷键已保留供系统使用。','reserved left Alt'],
  [[['AltRight',true],['AltRight',false]],'此快捷键已被占用，请换一个。','other purpose conflict']
 ]){
  const before=await invoke(page,'shortcut_status');await beginCapture();
  for(const [key,down] of events)await keyEvent(key,down);
  await expect(page.getByRole('alert').filter({hasText:message})).toBeVisible();
  await expect(page.getByRole('button',{name:'更改翻译快捷键',exact:true})).toHaveAttribute('aria-invalid','true');
  const after=await invoke(page,'shortcut_status');expect(after.saved).toEqual(before.saved);expect(after.active).toEqual(before.active);expect(after.capturing).toBe(false);
  report.captureProbe.cases.push({label,events,message,before,after});
 }
 await beginCapture('binding');
 await finishKeys([['F9',true],['F9',false]],'F9','ordinary binding captured','binding');
 await beginCapture('binding');
 await finishKeys([['AltRight',true],['AltRight',false]],'AltRight','ordinary binding restored by capture','binding');
 for(const [events,label] of [
  [[['AltRight',true],['ShiftRight',true],['ShiftRight',false],['AltRight',false]],'Alt then Shift'],
  [[[['AltRight','ShiftRight'],true],['AltRight',false],['ShiftRight',false]],'simultaneous chord']
 ]){await beginCapture();await finishKeys(events,'AltRight+ShiftRight',label);}
 report.captureProbe.after=await invoke(page,'shortcut_status');report.captureProbe.dom=await page.evaluate(()=>window.__captureEdges);
 await page.getByRole('button',{name:'关闭',exact:true}).click();
 await invoke(page,'runtime_window',{surface:'main',action:'hide'});
 } catch(error) { const value=await invoke(page,'shortcut_status');report.captureProbe??={};report.captureProbe.failureStatus=value;report.captureProbe.dom=await page.evaluate(()=>window.__captureEdges??[]);await rejected(page,'shortcut_capture',{begin:false,token:value.captureToken}); await editor.close(); throw error; }
 const pointerEvents=[];await overlay.exposeFunction('__translationPointer',event=>pointerEvents.push(event));
 await overlay.evaluate(()=>{
  for(const type of ['pointerdown','pointerup','click'])document.addEventListener(type,event=>{const item=event.target.closest?.('[role="menuitemradio"],button');void window.__translationPointer({type,text:item?.textContent,role:item?.getAttribute('role'),x:event.clientX,y:event.clientY});},true);
 });
 report.translation={requests:translationRequests,rounds:[],gestures:[],menuGeometry:[],pointerEvents,supplier:'isolated TLS fixture',microphone:'actual default device',target:'owned native TSF control'};
 const body=async()=>{const value=await editor.command({action:'read'});expect(value.foregroundProcess,'Only the authorized native target may own foreground').toBe(editor.pid);return value.text;};
 let gestureIndex=0;
 const translateTap=async()=>{
  const order=['AltRight','ShiftRight'];
  const sameBatch=gestureIndex%4>=2;
  const releaseOrder=gestureIndex%2?[...order]:[...order].reverse();
  const original=await invoke(page,'dictation_snapshot'),before=await editor.command({action:'read'});
  if(sameBatch){const value=await editor.command({action:'fixture-key',process:editor.pid,keys:order,down:true});expect(value.keyResult).toMatchObject({accepted:true,keys:[165,161],sent:2});}
  else for(const key of order){const value=await editor.command({action:'fixture-key',process:editor.pid,key,down:true});expect(value.keyResult?.accepted).toBe(true);}
  const held=await invoke(page,'dictation_snapshot');expect(held.taskId).toBe(original.taskId);expect(held.stage).toBe(original.stage);
  for(const key of releaseOrder){const value=await editor.command({action:'fixture-key',process:editor.pid,key,down:false});expect(value.keyResult?.accepted).toBe(true);}
  await expect.poll(async()=>(await editor.command({action:'read'})).modifiersDown,{timeout:2000}).toEqual([]);
  const after=await editor.command({action:'read'});expect(after.keyboardLayout).toBe(before.keyboardLayout);expect(after.menuActive).toBe(false);
  report.translation.gestures.push({order,sameBatch,releaseOrder,before:before.keyboardLayout,after:after.keyboardLayout,imeBefore:before.ime,imeAfter:after.ime,shiftUpBefore:before.shiftUpSequence,shiftUpAfter:after.shiftUpSequence,menuActive:after.menuActive,modifiersDown:after.modifiersDown});
  if(before.ime?.read&&after.ime?.read)expect(after.ime).toEqual(before.ime);
  gestureIndex++;
 };
 const settle=async task=>{
  await expect.poll(async()=>{const snapshot=await invoke(page,'dictation_snapshot');return snapshot.taskId===task&&snapshot.stage==='result'&&snapshot.delivery!==null&&!['recognizing','delivering','recording'].includes(snapshot.presentation);},{timeout:20000}).toBe(true);
  return invoke(page,'dictation_snapshot');
 };
 let failure;
 try{
  for(const name of ['success','same-batch','failure','cancel','target-change','hold']){
   mode='translation-'+name;
   if(name==='hold')await invoke(page,'shortcut_save',{config:{...(await invoke(page,'shortcut_status')).saved,translationBinding:'F8',mode:'hold'}});
   await editor.command({action:'focus',text:'前后',start:1,end:1});
   const beforeRequests=translationRequests.length;
   if(name==='hold'){const down=await editor.command({action:'fixture-key',process:editor.pid,key:'F8',down:true});expect(down.keyResult?.accepted).toBe(true);}else await translateTap();
   await waitPhase(page,'recording');
   const active=await invoke(page,'dictation_snapshot');expect(active.purpose).toBe('translation');
   await expect.poll(async()=>(await invoke(page,'speech_snapshot')).text,{timeout:8000}).toBe('正在识别');
   expect(await body()).toBe('前后');expect(translationRequests).toHaveLength(beforeRequests);
   if(name==='success'){
    const initialWindow=await editor.command({action:'inspect-overlay',process:app.pid});
    for(let toggle=0;toggle<6;toggle++){
     await nativeOverlayClick(overlay,editor,'翻译目标语言');await expect(overlay.getByRole('menu')).toBeVisible();
     const open=await editor.command({action:'inspect-overlay',process:app.pid});expect(open.overlayRect).toEqual(initialWindow.overlayRect);expect(open.foregroundProcess).toBe(editor.pid);
     await nativeOverlayClick(overlay,editor,'翻译目标语言');await expect(overlay.getByRole('menu')).toHaveCount(0);
     const closed=await editor.command({action:'inspect-overlay',process:app.pid});expect(closed.overlayRect).toEqual(initialWindow.overlayRect);expect(closed.foregroundProcess).toBe(editor.pid);
     report.translation.menuGeometry.push({open:open.overlayRect,closed:closed.overlayRect,foreground:closed.foregroundProcess});
    }
    await nativeOverlayClick(overlay,editor,'翻译目标语言');
    await expect(overlay.getByRole('menuitemradio',{name:'日语',exact:true})).toBeVisible();
    expect(await body()).toBe('前后');
    await nativeOverlayClick(overlay,editor,'日语','menuitemradio');
    await expect.poll(async()=>(await invoke(page,'dictation_snapshot')).targetLanguage).toBe('ja');
    expect(await body()).toBe('前后');
    await expect(overlay.getByRole('menu')).toHaveCount(0);await nativeOverlayClick(overlay,editor,'翻译目标语言');await expect(overlay.getByRole('menu')).toBeVisible();
    const blank=await editor.command({action:'click-window',process:app.pid,window:initialWindow.overlayWindow,x:2,y:2});
    report.translation.transparentArea={pointRootMatches:blank.clickCheck.pointRootMatches,clicked:blank.clicked};
    if(blank.clickCheck.pointRootMatches){
     await expect(overlay.getByRole('menu')).toHaveCount(0);expect(await body()).toBe('前后');
     await nativeOverlayClick(overlay,editor,'翻译目标语言');await expect(overlay.getByRole('menu')).toBeVisible();
     await nativeOverlayClick(overlay,editor,'日语','menuitemradio');await expect(overlay.getByRole('menu')).toHaveCount(0);
     expect((await invoke(page,'dictation_snapshot')).targetLanguage).toBe('ja');
    }
   }
   if(name==='hold'){const up=await editor.command({action:'fixture-key',process:editor.pid,key:'F8',down:false});expect(up.keyResult?.accepted).toBe(true);}else await translateTap();
   await expect.poll(()=>translationRequests.length,{timeout:12000}).toBe(beforeRequests+1);
   const request=translationRequests.at(-1);expect(request.source).toBe('原生受控识别完成。');expect(request.request.messages[0].content).toContain('Japanese');
   const frozen=await invoke(page,'dictation_snapshot');expect(frozen.translationSelectable).toBe(false);expect(await body()).toBe('前后');
   expect((await rejected(page,'translation_select',{taskId:active.taskId,code:'en-US'})).code).toBeTruthy();
   if(name==='target-change'){await editor.command({action:'selection',start:0,end:0});const selection=await editor.command({action:'read'});expect([selection.start,selection.end]).toEqual([0,0]);}
   if(name==='cancel')await invoke(page,'dictation_control',{taskId:active.taskId,action:'cancel'});
   translationReleases.get(request)?.();
   const result=await settle(active.taskId),actual=await body();
   const expected=['success','same-batch','hold'].includes(name)?'前午後3時に会議です。后':'前后';expect(actual).toBe(expected);
   expect(result.translationComplete).toBe(!['failure','cancel'].includes(name));
   expect(result.delivery.restoreError).toBe(false);
   if(['success','same-batch','hold'].includes(name)){expect(result.delivery.submission).toBe('submitted');expect(result.error).toBeNull();}
   else expect(result.delivery.submission).toBe('notSubmitted');
   if(name==='target-change')expect(result.delivery.error).toBe('targetChanged');
   if(['failure','cancel'].includes(name)){expect(result.error).toBe(name==='cancel'?'cancelled':'service');expect(result.text).toBe('原生受控识别完成。');}
   const speech=await invoke(page,'speech_snapshot');
   const history=await invoke(page,'history_detail',{id:speech.recordId});
   expect(history.attempts[0]).toMatchObject({purpose:'translation',targetLanguage:'ja',text:'原生受控识别完成。'});
   expect(history.processing).toHaveLength(1);expect(history.processing[0].units).toHaveLength(1);
   expect(history.processing[0].units[0].sourceText).toBe('原生受控识别完成。');
   expect(history.processing[0].units[0].generationState).toBe(name==='failure'?'failed':name==='cancel'?'interrupted':'success');
   expect(history.processing[0].units[0].applicationState).toBe(['success','same-batch','hold'].includes(name)?'applied':'notApplied');
   expect(translationRequests).toHaveLength(beforeRequests+1);
   report.translation.rounds.push({case:name,taskId:active.taskId,before:'前后',during:'前后',actual,expected,result,historyPurpose:history.attempts[0].purpose,target:history.attempts[0].targetLanguage,audioFrames:report.connections.at(-1).frames});
   if(result.presentationId)await dismiss(page,{taskId:active.taskId,presentationId:result.presentationId});
  }
  expect(report.translationLearningRequests??[]).toHaveLength(0);
  await invoke(page,'shortcut_save',{config:{...(await invoke(page,'shortcut_status')).saved,translationBinding:'AltRight+ShiftRight',mode:'toggle'}});
  for(let round=0;round<2;round++){
   mode='normal';const beforeOrdinary=report.ordinaryTranslationRegressionRequests?.length??0;await editor.command({action:'focus',text:'',start:0,end:0});
   const result=await externalRound(page,async()=>{await expect.poll(body,{timeout:8000}).toBe('正在识别');},undefined,false,async()=>{
    for(const down of [true,false]){const event=await editor.command({action:'fixture-key',process:editor.pid,key:'AltRight',down});expect(event.keyResult?.accepted).toBe(true);}
    return waitPhase(page,'recording');
   });
   expect(await body()).toBe('原生受控识别完成。');expect(result.dictation.purpose).toBe('dictation');expect(report.ordinaryTranslationRegressionRequests.length).toBeGreaterThan(beforeOrdinary);expect(report.modelError).toBeUndefined();
   report.translation.rounds.push({case:'ordinary-after-translation',round,actual:await body(),result:result.dictation});
   if(result.dictation.presentationId)await dismiss(page,{taskId:result.dictation.taskId,presentationId:result.dictation.presentationId});
  }
  report.checks.push('production translation transaction: final ASR tail, no raw insertion, actual nonactivating language click, freeze, one request/write, failure/cancel/target-change, hold release, history and two ordinary rounds');
 }catch(error){
  report.translation.firstFailure={message:String(error),stack:error.stack};
  failure=error;
 }finally{
  const clean=async(label,action)=>{try{await action();}catch(error){report.translation.cleanupFailures??=[];report.translation.cleanupFailures.push({label,error:String(error)});failure??=error;}};
  await clean('task',async()=>{const active=await invoke(page,'dictation_snapshot');if(active?.taskId)await rejected(page,'dictation_control',{taskId:active.taskId,action:'cancel'});});
  await clean('supplier',async()=>{for(const release of translationReleases.values())release();});
  await clean('editor',()=>editor.close());
 }
 if(failure)throw failure;
}
async function polishScenario(page,base){
 const overview=await readModels(page);
 const configured=await invoke(page,'model_save',{request:{id:null,revision:null,expectation:null,input:{name:'隔离整理验收',protocol:'openai-chat',baseUrl:base+'/v1',modelId:'polish-fixture',usages:['text']},key:{action:'replace',value:key}}});
 expect(configured.defaults.find(d=>d.usage==='speech').configId).toBe(overview.defaults.find(d=>d.usage==='speech').configId);
 const editor=await ownedEditor();mode='polish';
 try{
  if(polishCase==='cancel'){report.polish={};await polishRaces(page,editor,'cancel');return;}
  if(polishCase==='no-history'){
   const preferences=await invoke(page,'speech_preferences',{preferences:null});
   await invoke(page,'speech_preferences',{preferences:{...preferences,historyEnabled:false}});
   report.polish={};await polishRaces(page,editor,'cancel');
   const snapshot=await invoke(page,'speech_snapshot');expect(snapshot.recordId).toBeNull();
   expect((await invoke(page,'history_browse',{query:{search:'',cursor:null}})).total).toBe(0);
   report.checks.push('history-disabled external cancellation leaves no persistent record or blocked history read');return;
  }
  await editor.command({action:'focus',text:'',start:0,end:0});
  const task=await invoke(page,'dictation_start');expect(task.source).toBe('external');
  const samples=[];let observedTask=task.taskId;report.polish={taskId:task.taskId,samples};
  const observe=async()=>{const value=(await editor.command({action:'read'})).text;samples.push({taskId:observedTask,at:performance.now(),text:value});return value;};
  const release=async(text)=>{await expect.poll(()=>[...polishReleases.keys()].some(k=>k.includes(text)),{timeout:12000}).toBe(true);for(const [input,go]of polishReleases){if(input.includes(text))go();}};
  await expect.poll(observe,{timeout:12000,intervals:[50,100]}).toContain('呃，第一段啊。');
  await release('第一段');
  await expect.poll(observe,{timeout:12000,intervals:[50,100]}).toContain('第一段。');
  await expect.poll(observe,{timeout:12000,intervals:[50,100]}).toContain('嗯，第二段啊。');
  await release('第二段');
  await expect.poll(observe,{timeout:12000,intervals:[50,100]}).toContain('第二段。');
  await expect.poll(async()=>report.polishRequests.filter(r=>r.input.includes('第二段')).length,{timeout:5000}).toBe(1);
  await expect.poll(async()=>{const value=(await editor.command({action:'read'})).text;samples.push({taskId:observedTask,at:performance.now(),text:value});return value;},{timeout:5000}).toContain('尾部');
  expect(report.polishRequests.filter(r=>r.input.includes('第一段'))).toHaveLength(1);
  await invoke(page,'dictation_control',{taskId:task.taskId,action:'finish'});
  await expect.poll(async()=>(await editor.command({action:'read'})).text,{timeout:12000}).toBe('第一段。\r\n第二段。\r\n尾部完成🙂');
  await expect.poll(async()=>(await invoke(page,'dictation_snapshot')).presentation,{timeout:5000}).toBe('idle');
  const completed=await invoke(page,'speech_snapshot');const detail=await invoke(page,'history_detail',{id:completed.recordId});
  expect(detail.attempts.find(a=>a.id===completed.attemptId).text).toBe('呃，第一段啊。嗯，第二段啊。尾部完成🙂');
  expect(detail.processing[0].text).toBe('第一段。\n第二段。\n尾部完成🙂');
  const summary=await invoke(page,'history_browse',{query:{search:'',cursor:null}});expect(summary.records[0].displayReference.kind).toBe('processed');
  report.polish={taskId:task.taskId,samples,actualFinal:(await editor.command({action:'read'})).text,raw:detail.attempts[0].text,processed:detail.processing[0].text,units:detail.processing[0].units.map(u=>({generation:u.generationState,application:u.applicationState,sourceStart:u.sourceStart,sourceEnd:u.sourceEnd,committedEnd:u.sourceCommittedEnd})),supplierRequests:false};
  report.checks.push('controlled ASR partial then2stable LLM prefixes with live tail, exact native final multiline/emoji, raw/processed independent and summary reference bound');
  // Start a second real session; keep the next model response pending while
  // cancelling so persistence cannot retain unsubmitted generation payloads.
  const beforeRequests=report.polishRequests.length;
  await editor.command({action:'focus',text:'',start:0,end:0});
  const cancelled=await invoke(page,'dictation_start');observedTask=cancelled.taskId;
  await expect.poll(observe,{timeout:12000,intervals:[50,100]}).toContain('呃，第一段啊。');
  await release('第一段');
  await expect.poll(observe,{timeout:12000,intervals:[50,100]}).toContain('第一段。');
  await expect.poll(()=>[...polishReleases.keys()].some(k=>k.includes('第二段')),{timeout:12000}).toBe(true);
  await invoke(page,'dictation_control',{taskId:cancelled.taskId,action:'cancel'});
  await expect.poll(async()=>(await invoke(page,'dictation_snapshot')).presentation,{timeout:12000}).toBe('idle');
  expect((await editor.command({action:'read'})).text).toBe('第一段。');
  const cancelSpeech=await invoke(page,'speech_snapshot'),cancelRecord=await invoke(page,'history_detail',{id:cancelSpeech.recordId});
  expect(cancelRecord.processing[0].state).toBe('cancelled');expect(cancelRecord.processing[0].text).toBe('第一段。');
  expect(cancelRecord.processing[0].units).toHaveLength(1);expect(cancelRecord.processing[0].units[0].candidateText).toBe('第一段。');
  expect(cancelRecord.attempts[0].text).toBe('呃，第一段啊。');expect(cancelRecord.audio?.status==='ready').toBe(false);
  report.polish.cancel={taskId:cancelled.taskId,actual:(await editor.command({action:'read'})).text,units:cancelRecord.processing[0].units.length,requests:report.polishRequests.length-beforeRequests,audio:cancelRecord.audio?.status??null};
  report.checks.push('cancel after submitted prefix removes owned tail/separator, drops pending processing unit and discards recording');
  await polishRaces(page,editor);
 }finally{await editor.close();mode='normal';}
}
// Full production actor with real owned EDIT text. ASR/model timing is gated,
// not inferred from a native acknowledgement or a physical Codex observation.
async function polishRaces(page,editor,scope='all'){
 const actual=async()=>(await editor.command({action:'read'})).text;
 const pending=async(text)=>{let go;await expect.poll(()=>{go=[...polishReleases].find(([key])=>key.includes(text))?.[1];return Boolean(go);},{timeout:12000}).toBe(true);return go;};
 const finish=async(task)=>{await invoke(page,'dictation_control',{taskId:task.taskId,action:'finish'});await expect.poll(async()=>(await invoke(page,'dictation_snapshot')).presentation,{timeout:20000}).toBe('idle');};
 report.polish.races=[];

 if(scope==='all'){
 mode='polish-revision';await editor.command({action:'focus',text:'',start:0,end:0});
 const revised=await invoke(page,'dictation_start');const stableRelease=await pending('修订前');
 const stableRequest=report.polishRequests.at(-1);await expect.poll(actual,{timeout:12000}).toBe('嗯，修订前啊。');
 const channel=polishChannels.find(c=>c.mode===mode);channel.sentence(1,'嗯，修订后🙂。',true);
 // Qwen confirmed sentence IDs are immutable: a conflicting duplicate is
 // deliberately ignored by the adapter, not a fabricated actor revision.
 await delay(250);expect(await actual()).toBe('嗯，修订前啊。');
 expect(report.polishRequests.filter(r=>r.mode===mode)).toHaveLength(1);expect(stableRequest.aborted).not.toBe(true);
 stableRelease();await finish(revised);expect(await actual()).toBe('嗯，修订前啊。');
 report.polish.races.push({case:'confirmed-provider-duplicate',taskId:revised.taskId,actual:await actual(),generationRequests:1});

 mode='polish-edit';await editor.command({action:'focus',text:'',start:0,end:0});
 const edited=await invoke(page,'dictation_start');await pending('编辑前');
 const oldTurnRequest=report.polishRequests.at(-1);await expect.poll(actual,{timeout:12000}).toBe('呃，编辑前啊。');
 for(const down of [true,false])await editor.command({action:'fixture-key',key:'Backspace',down,process:editor.pid});
 await expect.poll(actual,{timeout:4000}).toBe('呃，编辑前啊');
 await expect.poll(()=>polishChannels.filter(c=>c.mode==='polish-edit').length,{timeout:12000}).toBe(2);
 const nextTurn=await pending('继续输入');await expect.poll(()=>oldTurnRequest.aborted,{timeout:4000}).toBe(true);
 nextTurn();await expect.poll(actual,{timeout:12000}).toBe('呃，编辑前啊继续输入🙂。');await finish(edited);
 expect(await actual()).toBe('呃，编辑前啊继续输入🙂。');
 report.polish.races.push({case:'edit-during-generation',taskId:edited.taskId,actual:await actual(),oldGenerationAborted:true,retiredFinalExcluded:true,injectedKeyboard:true});

 mode='polish-timeout';await editor.command({action:'focus',text:'',start:0,end:0});
 const timed=await invoke(page,'dictation_start');await pending('等待原文');const timedRequest=report.polishRequests.at(-1);
 await expect.poll(actual,{timeout:12000}).toBe('呃，等待原文啊。');const began=performance.now();await finish(timed);
 expect(await actual()).toBe('呃，等待原文啊。');await expect.poll(()=>timedRequest.aborted,{timeout:4000}).toBe(true);
 const timedSpeech=await invoke(page,'speech_snapshot'),timedDetail=await invoke(page,'history_detail',{id:timedSpeech.recordId});
 expect(timedDetail.processing[0].units.some(u=>u.errorCode==='timeout'&&u.generationState==='failed')).toBe(true);
 report.polish.races.push({case:'generation-timeout',taskId:timed.taskId,actual:await actual(),finishMs:Math.round(performance.now()-began),generationAborted:true});
 }

 mode='polish-abort';await editor.command({action:'focus',text:'甲乙',start:0,end:1});
 const aborted=await invoke(page,'dictation_start');await pending('等待原文');const abortRequest=report.polishRequests.at(-1);
 await expect.poll(actual,{timeout:12000}).toBe('呃，等待原文啊。乙');
 await invoke(page,'dictation_control',{taskId:aborted.taskId,action:'cancel'});
 await expect.poll(async()=>(await invoke(page,'dictation_snapshot')).presentation,{timeout:12000}).toBe('idle');
 const restored=await editor.command({action:'read'});expect(restored.text).toBe('甲乙');await expect.poll(()=>abortRequest.aborted,{timeout:4000}).toBe(true);
 report.polish.races.push({case:'cancel-pending-generation',taskId:aborted.taskId,actual:restored.text,originalTextRestored:true,observedSelection:{start:restored.start,end:restored.end},generationAborted:true});
 report.checks.push(scope==='cancel'?'targeted production actor cancel pending generation: original text restored, selection observed, generation aborted':'full actor actual-text immutable confirmed-provider duplicate, Backspace handoff/new ASR turn with obsolete generation+retired final,15s generation timeout raw fallback and cancel restores original text');
}
async function processMetrics(){
 const taskPid=app.pid;if(!Number.isSafeInteger(taskPid)||taskPid<=0)throw Error('Invalid owned process');
 const code=`$taskProcess=Get-Process -Id ${taskPid}; if ($taskProcess.Path -ne '${exe.replaceAll("'","''")}') {throw 'Process ownership changed'}; [pscustomobject]@{cpuSeconds=$taskProcess.TotalProcessorTime.TotalSeconds;workingSetBytes=$taskProcess.WorkingSet64;privateBytes=$taskProcess.PrivateMemorySize64} | ConvertTo-Json -Compress`;
 const probe=spawn('powershell',['-NoProfile','-Command',code],{windowsHide:true,stdio:['ignore','pipe','pipe']});let output='';probe.stdout.on('data',chunk=>output+=chunk);if(await new Promise((r,j)=>{probe.on('exit',r);probe.on('error',j);})!==0)throw Error('Owned process metrics unavailable');return JSON.parse(output);
}
const invoke=async(page,command,args={})=>{const result=await page.evaluate(async({command,args})=>{try{return {value:await window.__TAURI_INTERNALS__.invoke(command,args)};}catch(e){return {error:e.code??'unavailable'};}},{command,args});if(result.error)throw Error(command+':'+result.error);return result.value;};
async function readModels(page){for(let n=0;n<30;n++){try{const status=await invoke(page,'model_status',{retry:false});if(status.state==='preparing'){await delay(100);continue;}expect(status.state).toBe('ready');return await invoke(page,'model_overview');}catch(e){if(n===29||!/:busy$|:preparing$/.test(String(e)))throw e;await delay(100);}}throw Error('Model initialization did not become ready');}
const rejected=(page,command,args={})=>page.evaluate(async({command,args})=>{try{await window.__TAURI_INTERNALS__.invoke(command,args);return null;}catch(e){return e;}},{command,args});
async function dismiss(page,binding){
 const error=await rejected(page,'dictation_dismiss',binding);
 if(!error)return;
 expect(error.code).toBe('staleSession');
 // Completion may auto-hide while actual target text is read. Only that exact
 // already-hidden task is acceptable; a different/current presentation is not.
 const latest=await invoke(page,'dictation_snapshot');
 expect(latest.taskId).toBe(binding.taskId);expect(latest.presentationId).toBeNull();expect(latest.presentation).toBe('idle');
}
async function start(round){
 if(await fetch('http://127.0.0.1:'+port+'/json/version').then(()=>true).catch(()=>false))throw Error('Test port occupied');
 app=spawn(exe,[],{cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:id,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+port+' --remote-debugging-address=127.0.0.1'},stdio:['ignore','pipe','pipe']});app.stdout.pipe(createWriteStream(path.join(directory,`round-${round}.log`)));app.stderr.pipe(createWriteStream(path.join(directory,`round-${round}-errors.log`)));
 report.processBindings??=[];report.processBindings.push({round,pid:app.pid,exe,sha256:createHash('sha256').update(readFileSync(exe)).digest('hex'),data,logSource:'launched-process-stderr'});
 let diagnostic='';app.stderr.on('data',chunk=>{diagnostic+=chunk.toString('utf8');let at;while((at=diagnostic.indexOf('\n'))>=0){const line=diagnostic.slice(0,at);diagnostic=diagnostic.slice(at+1);if(/operation=(input_target_event|input_target_identity|input_capture|input_delivery|input_binding|voice_window|dictation_phase|dictation_presentation|draft_presentation|speech_phase|speech_turn|draft_activation|composition_edit|dictation_turn)\b/.test(line)){report.taskTrace??=[];report.taskTrace.push({round,line});}}});
 let endpoint;for(let n=0;n<200;n++){if(app.exitCode!==null)throw Error('Application exited early');endpoint=await fetch('http://127.0.0.1:'+port+'/json/version').then(r=>r.json()).catch(()=>null);if(endpoint)break;await delay(100);}if(!endpoint)throw Error('WebView unavailable');browser=await chromium.connectOverCDP(endpoint.webSocketDebuggerUrl);
 let page;for(let n=0;n<100;n++){page=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('tauri')&&!p.url().includes('surface=overlay'));if(page)break;await delay(100);}if(!page)throw Error('Main page missing');await page.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke);for(let n=0;n<50;n++){const status=await invoke(page,'history_status',{retry:false});if(status.state==='ready')break;await delay(100);}await overlayPage();return page;
}
async function overlayPage(){
 let overlay;
 await expect.poll(()=>{overlay=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('surface=overlay')&&!p.isClosed());return Boolean(overlay);},{timeout:10000}).toBe(true);
 await overlay.waitForFunction(()=>window.__TAURI_INTERNALS__?.invoke);
 return overlay;
}
async function stop(page,graceful=true){
 if(app?.exitCode===null&&page&&graceful){await invoke(page,'acceptance_exit').catch(()=>{});for(let n=0;n<60&&app.exitCode===null;n++)await delay(100);if(app.exitCode===null)throw Error('Exit did not drain');}
 if(browser){await browser.close().catch(()=>{});browser=null;}if(app?.exitCode===null){const owned=app;owned.kill();await new Promise(r=>owned.exitCode!==null?r():owned.once('exit',r));}app=null;await delay(600);
}
async function ownedEditor(composition=true){
 const child=spawn(path.join(root,'src-tauri/target/debug/input-fixture.exe'),composition?['composition']:[],{cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_INPUT_ACCEPTANCE:'1'},stdio:['pipe','pipe','pipe']});
 const trace={pid:child.pid,commands:[],stderr:'',exit:null};report.ownedEditors??=[];report.ownedEditors.push(trace);
 child.stderr.on('data',chunk=>{trace.stderr=(trace.stderr+chunk.toString('utf8')).slice(-4096);});
 let buffer='',queue=[],waiting=[],channelFailure;
 const exited=new Promise(resolve=>child.once('exit',(code,signal)=>{trace.exit={code,signal};channelFailure??=Error('Owned editor exited before response');for(const waiter of waiting.splice(0))waiter.reject(channelFailure);resolve();}));
 child.stdout.on('data',data=>{buffer+=data.toString('utf8');let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at);buffer=buffer.slice(at+1);const value=JSON.parse(line);if(channelFailure)continue;if(waiting.length)waiting.shift().resolve(value);else queue.push(value);}});
 const next=()=>channelFailure?Promise.reject(channelFailure):queue.length?Promise.resolve(queue.shift()):new Promise((resolve,reject)=>{
  const waiter={resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}};
  const timer=setTimeout(()=>{waiting=waiting.filter(value=>value!==waiter);channelFailure=Error('Owned editor response timeout');reject(channelFailure);},3000);waiting.push(waiter);
 });
 await next();return {pid:child.pid,command:async value=>{if(channelFailure)throw channelFailure;const entry={request:structuredClone(value),at:performance.now()};trace.commands.push(entry);child.stdin.write(JSON.stringify(value)+'\n');const result=await next();entry.response=result;entry.completedAt=performance.now();if(['focus','focus-process'].includes(value.action))await delay(80);return result;},close:async()=>{if(child.exitCode===null){child.stdin.write('{"action":"exit"}\n');await Promise.race([exited,delay(3000)]);}expect(child.exitCode,'Owned fixture cleanup must finish successfully; unresolved resources remain retained').toBe(0);}};
}
async function nativeOverlayClick(view,editor,name,role='button'){
 const owned=await editor.command({action:'inspect-overlay',process:app.pid});expect(owned.overlayWindow).toBeGreaterThan(0);
 const item=view.getByRole(role,{name,exact:true});await item.click({trial:true});
 const box=await item.boundingBox();expect(box).not.toBeNull();
 const scale=await view.evaluate(()=>devicePixelRatio);
 const click=await editor.command({action:'click-window',process:app.pid,window:owned.overlayWindow,x:Math.round((box.x+box.width/2)*scale),y:Math.round((box.y+box.height/2)*scale)});
 expect(click.clickCheck.pointRootMatches).toBe(true);expect(click.clicked).toBe(2);return click;
}
async function externalRound(page,finish,end,interrupted=false,begin){
 const start=begin?await begin():await invoke(page,'dictation_start');await waitPhase(page,'recording');
 for(const command of ['dictation_control','speech_control'])expect((await rejected(page,command,{taskId:'stale-task',action:'cancel'})).code).toBe('staleSession');
 await finish(start);
 if(!['completed','failed','cancelled'].includes((await invoke(page,'speech_snapshot')).phase)){
  if(end)await end(start);else await invoke(page,'dictation_control',{taskId:start.taskId,action:'finish'});
 }
 if(mode==='delayed-final'){await expect.poll(async()=>(await invoke(page,'dictation_snapshot')).presentation,{timeout:3000}).toBe('recognizing');const view=await overlayPage();await expect(view.getByRole('group',{name:'识别中',exact:true})).toBeVisible();await expect(view.locator('.voiceControl_feedback')).toHaveCount(0);await expect(view.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);}
 if(interrupted)await expect.poll(async()=>['completed','failed','cancelled'].includes((await invoke(page,'speech_snapshot')).phase),{timeout:15000}).toBe(true);else await waitPhase(page,'completed');
 await expect.poll(async()=>{const result=await invoke(page,'dictation_snapshot');return result.taskId===start.taskId&&result.stage==='result'&&result.delivery!==null&&!['preparing','recording','recognizing','delivering'].includes(result.presentation);},{timeout:5000}).toBe(true);
 return {speech:await invoke(page,'speech_snapshot'),dictation:await invoke(page,'dictation_snapshot')};
}
async function waitPhase(page,phase){
 let timeout=15000;
 if(phase==='completed'){
  const snapshot=await invoke(page,'speech_snapshot');
  if(snapshot.retry){
   // Replay sends at real-time speed; preparation and final-result waits have separate budgets.
   const duration=snapshot.recordId?(await invoke(page,'history_detail',{id:snapshot.recordId})).durationMs:snapshot.durationMs;
   timeout=duration+15000+30000;
  }
 }
 await expect.poll(async()=>(await invoke(page,'speech_snapshot')).phase,{timeout}).toBe(phase);return invoke(page,'speech_snapshot');
}
async function resultOnlyScenario(page){
 const helper=await ownedEditor(),overlay=await overlayPage();
 const targetBrowser=await chromium.launch({headless:false,args:['--disable-features=CalculateNativeWinOcclusion']});
 try{
  const session=await targetBrowser.newBrowserCDPSession();
  const process=(await session.send('SystemInfo.getProcessInfo')).processInfo.find(p=>p.type==='browser').id;await session.detach();
  const target=await targetBrowser.newPage();
  await target.setContent('<main tabindex="0" id="plain">只读示例页面</main><button id="button">普通按钮</button><textarea id="readonly" readonly>只读正文</textarea><input id="password" type="password" value="protected"/><textarea id="editable">块外｜</textarea>');
  const bodies=()=>target.evaluate(()=>({plain:document.querySelector('#plain').textContent,readonly:document.querySelector('#readonly').value,editable:document.querySelector('#editable').value,password:document.querySelector('#password').value}));
  const stableBodies=()=>target.evaluate(async()=>{
   let previous='',stable=0;const samples=[];
   for(let n=0;n<60;n++){
    await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
    const text={plain:document.querySelector('#plain').textContent,readonly:document.querySelector('#readonly').value,editable:document.querySelector('#editable').value,password:document.querySelector('#password').value};
    const next=JSON.stringify(text);stable=next===previous?stable+1:0;previous=next;samples.push(text);
    if(stable>=2)return {text,samples};
   }
   throw Error('Owned target body did not settle');
  });
  const focus=async id=>{
   await target.bringToFront();await target.locator('#'+id).focus();
   expect((await helper.command({action:'focus-process',process})).foregroundProcess).toBe(process);
  };
  report.resultOnly=[];
  let priorBinding;
  const copyRound=async(name,prepare,{release=true,cancel=false,moveFocus=true,copy=false}={})=>{
   await prepare();const before=await bodies(),clipboard=(await helper.command({action:'read'})).clipboardSequence;
   const count=report.connections.length;mode='copy-only';const started=await invoke(page,'dictation_start');
   await waitPhase(page,'recording');
   const admission=await invoke(page,'dictation_snapshot');expect(admission.taskId).toBe(started.taskId);expect(admission.inputRoute).toBe('resultOnly');expect(admission.admission).toBe('ready');
   await expect.poll(()=>report.connections.length,{timeout:5000}).toBe(count+1);
   const connection=report.connections.at(-1);await expect.poll(()=>connection.frames,{timeout:5000}).toBeGreaterThan(0);
   // A deliberately gated provider sends no text until startup has proved that
   // this task has no write owner. Focus changes cannot promote it to live input.
   if(moveFocus)await focus('editable');
   if(priorBinding)expect((await rejected(page,'dictation_copy',priorBinding)).code).toBe('staleSession');
   if(release){const go=copyReleases.get(connection);expect(typeof go).toBe('function');go();await expect.poll(async()=>(await invoke(page,'speech_snapshot')).text).toBe('正在识别');}
   await invoke(page,'dictation_control',{taskId:started.taskId,action:cancel?'cancel':'finish'});
   await waitPhase(page,cancel?'cancelled':release?'completed':'failed');
   await expect.poll(async()=>{const s=await invoke(page,'dictation_snapshot');return s.taskId===started.taskId&&s.stage==='result'&&!['preparing','recording','recognizing','delivering'].includes(s.presentation);},{timeout:5000}).toBe(true);
   const result=await invoke(page,'dictation_snapshot'),speech=await invoke(page,'speech_snapshot');
   const actual=await stableBodies();expect(actual.text).toEqual(before);
   expect((await helper.command({action:'read'})).clipboardSequence).toBe(clipboard);
   if(release&&!cancel){
    expect(result.presentation).toBe('result');expect(result.text).toBe('原生受控识别完成。');
    expect(result.delivery.state).toBe('copy');expect(result.delivery.error).toBeNull();expect(result.delivery.submission).toBe('notSubmitted');expect(result.delivery.verification).toBe('notAttempted');
    await expect(overlay.getByRole('button',{name:'复制',exact:true})).toBeVisible();await expect(overlay.getByRole('button',{name:'关闭',exact:true})).toBeVisible();
    await expect.poll(async()=>(await invoke(page,'history_delivery',{id:speech.recordId}))[0]?.state,{timeout:5000}).toBe('copy');
    expect((await invoke(page,'history_delivery',{id:speech.recordId}))[0].error).toBeNull();
    priorBinding={taskId:result.taskId,presentationId:result.presentationId};
    if(copy){
     await nativeOverlayClick(overlay,helper,'复制');
     await expect.poll(async()=>(await helper.command({action:'read'})).clipboardSequence,{timeout:3000}).toBeGreaterThan(clipboard);
     // Inspect only equality with this owned fixture text. Private clipboard
     // contents are never returned or included in a report.
     const code="[pscustomobject]@{matches=((Get-Clipboard -Raw) -eq '原生受控识别完成。')} | ConvertTo-Json -Compress";
     const child=spawn('powershell.exe',['-NoProfile','-NonInteractive','-EncodedCommand',Buffer.from(code,'utf16le').toString('base64')],{windowsHide:true,stdio:['ignore','pipe','pipe']});let output='';child.stdout.on('data',chunk=>output+=chunk);
     expect(await new Promise((resolve,reject)=>{child.on('exit',resolve);child.on('error',reject);})).toBe(0);expect(JSON.parse(output).matches).toBe(true);
    }
    await dismiss(page,priorBinding);
   }else{expect(result.presentation).toBe('idle');await expect(overlay.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);}
   copyReleases.delete(connection);report.resultOnly.push({name,taskId:started.taskId,admission:admission.inputRoute,frames:connection.frames,delivery:result.delivery,presentation:result.presentation,actual,clipboardUnchangedBeforeCopy:true,explicitCopyVerified:copy});
  };
  await copyRound('own-main',async()=>{await invoke(page,'runtime_window',{surface:'main',action:'show'});expect((await helper.command({action:'focus-process',process:app.pid})).foregroundProcess).toBe(app.pid);},{copy:true});
  await copyRound('desktop',async()=>{const state=await helper.command({action:'focus-desktop'});expect(state.foregroundWindow).toBe(state.desktopWindow);});
  for(const id of ['plain','button','readonly'])await copyRound(id,()=>focus(id));
  await copyRound('empty',()=>focus('readonly'),{release:false});
  await copyRound('cancel',()=>focus('readonly'),{cancel:true});
  await focus('password');const count=report.connections.length;
  expect((await rejected(page,'dictation_start')).code).toBe('targetPassword');expect(report.connections.length).toBe(count);
  const refusal=await invoke(page,'dictation_snapshot');expect(refusal.admission).toBe('rejected');await dismiss(page,{taskId:refusal.taskId,presentationId:refusal.presentationId});
  await focus('editable');await target.locator('#editable').evaluate(node=>node.setSelectionRange(node.value.length,node.value.length));mode='normal';
  const live=await externalRound(page,async()=>{await expect.poll(()=>target.locator('#editable').inputValue()).toBe('块外｜正在识别');});
  expect(live.dictation.inputRoute).toBe('live');expect(live.dictation.delivery.submission).toBe('submitted');
  const actual=await stableBodies();expect(actual.text.editable).toBe('块外｜原生受控识别完成。');
  report.resultOnlyLiveControl={actual,delivery:live.dictation.delivery};
  report.checks.push('positive no-target startup records with microphone frames; fixed copy route across focus changes; result/empty/cancel and stale-copy identity; password rejection; editable live control actual text');
 }finally{await targetBrowser.close();await helper.close();}
}
async function homeTrialIsolation(page){
 const overlay=await overlayPage(),editor=await ownedEditor();
 const practice=page.getByRole('region',{name:'识别文字',exact:true});
 const empty=await practice.textContent();
 try{
  async function external(expectedHome){
   await editor.command({action:'focus',text:'prefix ',start:7,end:7});
   const result=await externalRound(page,async()=>{
    expect((await invoke(page,'speech_snapshot')).source).toBe('external');
    await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('prefix 正在识别');
    await expect(practice).toHaveText(expectedHome);
    await expect(page.getByRole('button',{name:'开始录音',exact:true})).toBeDisabled();
    await expect(page.getByRole('button',{name:'结束录音',exact:true})).toHaveCount(0);
    await expect(page.getByRole('button',{name:'重新识别',exact:true})).toHaveCount(0);
   });
   expect(result.speech.source).toBe('external');
   expect((await editor.command({action:'read'})).text).toBe('prefix 原生受控识别完成。');
   await expect(practice).toHaveText(expectedHome);
   await expect.poll(async()=>(await invoke(page,'runtime_snapshot')).windows.overlay,{timeout:3000}).toBe(false);
   await expect(overlay.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);
   report.checks.push('actual TSF external text inserted while Home trial text/phase/meter remained isolated');
  }
  await external(empty);
  await invoke(page,'runtime_window',{surface:'main',action:'show'});
  await page.getByRole('button',{name:'开始录音',exact:true}).click();
  const trial=await waitPhase(page,'recording');expect(trial.source).toBe('trial');
  await expect(practice).toContainText('正在识别');
  await page.getByRole('button',{name:'结束录音',exact:true}).click();
  const done=await waitPhase(page,'completed');expect(done.source).toBe('trial');expect(done.saved).toBe(true);
  const retained=await practice.textContent();
  await page.getByRole('group',{name:'识别完成',exact:true}).getByRole('button',{name:'开始录音',exact:true}).click();
  const restarted=await waitPhase(page,'recording');expect(restarted.source).toBe('trial');expect(restarted.taskId).not.toBe(done.taskId);
  await page.getByRole('button',{name:'结束录音',exact:true}).click();
  const restartedDone=await waitPhase(page,'completed');
  const model=(await readModels(page)).configs[0];
  const retried=await invoke(page,'speech_retry_current',{taskId:restartedDone.taskId});expect(retried.source).toBe('trial');
  await waitPhase(page,'completed');await waitPhase(page,'idle');
  await external(retained);
  mode='failure';await invoke(page,'history_retry',{id:done.recordId,configId:model.id,configRevision:model.revision});
  const failed=await waitPhase(page,'failed');expect(failed.source).toBe('history');
  await expect(practice).toHaveText(retained);await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.getByRole('button',{name:'重新识别',exact:true})).toHaveCount(0);
  await page.getByRole('link',{name:'历史记录',exact:true}).click();
  await page.getByRole('link',{name:'首页',exact:true}).click();await expect(practice).toHaveText(retained);
  await invoke(page,'runtime_window',{surface:'main',action:'hide'});
  await invoke(page,'runtime_window',{surface:'main',action:'show'});await expect(practice).toHaveText(retained);
  await invoke(page,'speech_start');
  const ownFailure=await waitPhase(page,'failed');expect(ownFailure.source).toBe('trial');
  await expect(page.getByRole('alert')).toHaveCount(1);
  report.homeTrial={initialExternalIgnored:true,ownStreaming:true,completedRestartActualClick:true,retrySourcePreserved:true,historyFailureIgnored:true,navigationRetained:true,originalUserDataUsed:false};
  report.checks.push('Home active trial and current retry carry trial source; history errors, navigation and hide/resync preserve trial result');
 }finally{await editor.close();}
}
async function learningRestartScenario(main,base){
 const helper=await ownedEditor();
 const targetBrowser=await chromium.launch({headless:false,args:['--force-renderer-accessibility']});
 const pid=app.pid;
 const evidence={passed:false,applicationProcess:pid,trials:[],lifecycle:[],inputEvents:[],cleanupErrors:[]};
 report.learningRestart=evidence;
 let logBuffer='';
 const logListener=chunk=>{
  logBuffer+=chunk.toString('utf8');let at;
  while((at=logBuffer.indexOf('\n'))>=0){
   const line=logBuffer.slice(0,at);logBuffer=logBuffer.slice(at+1);
   if(/operation=(observation_retire|observation_poll|learning_field|composition_retired|composition_edit|input_composition_guard|dictation_first_error|dictation_phase|processing_dispatch|draft_delivery_detached|archive_settled|input_edit)\b/.test(line))evidence.lifecycle.push({at:Date.now(),line});
  }
 };
 app.stderr.on('data',logListener);
 let activeTask,target,scenarioError;
 try{
  await invoke(main,'model_save',{request:{id:null,revision:null,expectation:null,input:{name:'连续口述受控整理',protocol:'openai-chat',baseUrl:base+'/v1',modelId:'polish-fixture',usages:['text']},key:{action:'replace',value:key}}});
  const keys=await invoke(main,'shortcut_status');
  await invoke(main,'shortcut_save',{config:{...keys.saved,binding:'AltRight',mode:'toggle'}});
  evidence.shortcuts=await invoke(main,'shortcut_status');
  evidence.components=await invoke(main,'text_service_status');
  expect(evidence.components).toMatchObject({installed:true,enabled:true,payloadAvailable:true,updateAvailable:false,error:null});
  const session=await targetBrowser.newBrowserCDPSession();
  const editorPid=(await session.send('SystemInfo.getProcessInfo')).processInfo.find(p=>p.type==='browser').id;
  await session.detach();evidence.editorProcess=editorPid;evidence.browserVersion=targetBrowser.version();
  const bundle=await buildFixture({configFile:false,logLevel:'error',define:{'process.env.NODE_ENV':JSON.stringify('production')},build:{write:false,lib:{entry:path.join(root,'tests/fixtures/input-target.ts'),name:'InputFixture',formats:['iife']}}});
  const newTarget=async()=>{
  target=await targetBrowser.newPage();
  await target.setContent('<title>Sayweave 连续两轮口述验证</title><p id="instruction">正在准备同进程输入验证</p><input id="input"><textarea id="area"></textarea><div id="editable" contenteditable="true"></div>');
  await target.addScriptTag({content:(Array.isArray(bundle)?bundle[0]:bundle).output[0].code});
  await target.evaluate(()=>{
   window.continuityEvents=[];
   for(const type of ['keydown','keyup','compositionstart','compositionupdate','compositionend','beforeinput','input'])document.addEventListener(type,event=>{
    if(!event.target.closest?.('#prosemirror'))return;
    const record=phase=>window.continuityEvents.push({type,phase,key:event.key,code:event.code,data:event.data,inputType:event.inputType,at:Date.now(),body:window.fixtureEditor.read(),state:window.fixtureEditor.inspect()});
    record('capture');queueMicrotask(()=>record('microtask'));setTimeout(()=>record('after-task'),0);
   },true);
  });
  };
  const settled=()=>target.evaluate(async()=>{
   let previous='',stable=0;const samples=[],deadline=performance.now()+2000;
   while(performance.now()<deadline){
    await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
    const sample={text:window.fixtureEditor.read(),state:window.fixtureEditor.inspect(),eventCount:window.continuityEvents.length};
    const next=JSON.stringify(sample);stable=next===previous?stable+1:0;previous=next;samples.push(sample);
    if(stable>=2)return {...sample,samples};
   }
   throw Error('Main-process owned body did not settle');
  });
  const keyEvent=async(code,down)=>{
   const result=await helper.command({action:'fixture-key',process:editorPid,key:code,down});
   if(!result.keyResult?.accepted)throw Error('Owned main-process key refused: '+code);
  };
  const tap=async code=>{await keyEvent(code,true);await keyEvent(code,false);};
  const body=async(trial,label,expected)=>{
   const deadline=Date.now()+6000;let actual;
   do{
    actual=await settled();trial.bodies.push({at:Date.now(),label,...actual});
    if(actual.text===expected)return actual;
    await delay(50);
   }while(Date.now()<deadline);
   trial.firstDivergence={label,expected,actual:actual.text};
   throw Error(label+': actual body differs');
  };
  const startTask=async(trial,label)=>{
   expect(app.pid).toBe(pid);expect(app.exitCode).toBeNull();
   expect((await helper.command({action:'read'})).foregroundProcess).toBe(editorPid);
   const prior=(await invoke(main,'dictation_snapshot')).taskId;
   await tap('AltRight');
   const deadline=Date.now()+10000;let state;
   do{
    state=await invoke(main,'dictation_snapshot');
    if(state.taskId&&state.taskId!==prior){activeTask=state.taskId;if(state.presentation==='recording')break;}
    if(state.taskId!==prior&&state.error)break;
    await delay(50);
   }while(Date.now()<deadline);
   trial.tasks.push({label,at:Date.now(),snapshot:state,applicationProcess:app.pid,body:await settled()});
   if(state.taskId===prior||state.presentation!=='recording'||state.inputRoute!=='live')throw Error(label+': real shortcut did not admit live recording');
   return state.taskId;
  };
  const finishTask=async(trial,label)=>{
   await tap('AltRight');await waitPhase(main,'completed');
   await expect.poll(async()=>{const s=await invoke(main,'dictation_snapshot');return s.taskId===activeTask&&s.delivery!==null;},{timeout:10000}).toBe(true);
   const state=await invoke(main,'dictation_snapshot');trial.tasks.push({label,at:Date.now(),snapshot:state,body:await settled()});activeTask=null;
   if(state.error||state.inputRoute!=='live'||state.delivery?.restoreError||state.delivery?.submission==='uncertain')throw Error(label+': unsafe terminal delivery');
  };
  const failures=[];
  for(const enabled of [false,true]){
   const settings=await invoke(main,'language_settings');
   evidence.settings=await invoke(main,'language_save',{settings:{automaticTerms:{enabled,revision:settings.automaticTerms.revision},personalization:{enabled,revision:settings.personalization.revision}}});
   const trial={learning:enabled,bodies:[],tasks:[],began:Date.now(),cleanupErrors:[]};evidence.trials.push(trial);
   if(target){await target.close();trial.independentPairNewDocument=true;}
   await newTarget();
   try{
   await target.evaluate(()=>{window.fixtureEditor.set('',0,0);document.querySelector('#instruction').textContent='第一轮口述由测试驱动完成，请等待纠词确认提示。';});
   await target.bringToFront();await target.locator('#prosemirror .ProseMirror').focus();
   expect((await helper.command({action:'focus-process',process:editorPid})).foregroundProcess).toBe(editorPid);
   if(process.env.SAYWEAVE_LEARNING_RESTART_CORRECTION==='keyboard-replay'){
    // Align the IME before either first round; never reset it between the two rounds.
    trial.initialImeProbes=[];
    for(let attempt=0;attempt<2;attempt++){
     await tap('KeyQ');const probe=await body(trial,'IME setup probe','q');trial.initialImeProbes.push(probe);
     if(probe.state.composing){await tap('Escape');trial.initialImeAligned=true;break;}
     await tap('Backspace');await body(trial,'IME setup probe removed','');await tap('ShiftLeft');
    }
    expect(trial.initialImeAligned).toBe(true);
    const aligned=await body(trial,'aligned empty Chinese IME','');expect(aligned.state.composing).toBe(false);
   }
   mode='learning-restart-'+(enabled?'on':'off')+'-first';
   await startTask(trial,'first start');await body(trial,'first speech','Seville');
   await finishTask(trial,'first finish');await body(trial,'after first finish','Seville');
   const eventStart=await target.evaluate(()=>window.continuityEvents.length);
   await target.locator('#instruction').evaluate((node,enabled)=>{node.textContent='学习'+(enabled?'开启':'关闭')+'对照：请按平时方式逐字删除 Seville，手动输入 sayweave，再按 Shift 确认。随后测试会自动发送原 Alt 快捷键开始下一轮，请保持输入框焦点。';},enabled);
   const ready={at:Date.now(),learning:enabled,applicationProcess:pid,editorProcess:editorPid,eventStart};
   writeFileSync(path.join(directory,'shift-ready.json'),JSON.stringify(ready,null,2));
   console.log('CONTINUITY_SHIFT_READY '+JSON.stringify({...ready,directory}));
   const replayCorrection=process.env.SAYWEAVE_LEARNING_RESTART_CORRECTION==='keyboard-replay';
   if(replayCorrection){
    // Replay the recorded physical sequence through the owned foreground helper.
    // This is test stimulus; the application never confirms a foreign composition.
    for(let i=0;i<7;i++)await tap('Backspace');
    for(const letter of 'SAYWEAVE'){await tap('Key'+letter);await delay(40);}
    await tap('ShiftLeft');
   }
   // Shift is the owner operation. Native composition metadata is
   // recorded evidence, never substituted for whether the owner submitted.
   await target.waitForFunction(start=>window.continuityEvents.slice(start).some(e=>e.type==='keyup'&&['ShiftLeft','ShiftRight'].includes(e.code)&&e.body==='sayweave'),eventStart,{timeout:180000});
   trial.submitEvents=await target.evaluate(start=>window.continuityEvents.slice(start),eventStart);
   trial.correctionInput=replayCorrection?'owned keyboard replay of recorded delete/retype/Shift':'physical owner delete/retype/Shift';
   trial.beforeShift=trial.submitEvents.findLast(e=>e.type==='keydown'&&['ShiftLeft','ShiftRight'].includes(e.code)&&e.body==='sayweave');
   trial.afterShift=await body(trial,'Shift correction confirmation','sayweave');
   mode='learning-restart-'+(enabled?'on':'off')+'-next';
   await startTask(trial,'next start');await body(trial,'after next preparation','sayweave');
   await expect.poll(()=>{const c=restartChannels.at(-1)?.connection;return c?.mode===mode&&c.started&&c.frames>0;},{timeout:6000}).toBe(true);
   const channel=restartChannels.at(-1);
   expect(channel.connection.mode).toBe(mode);
   channel.sentence(1,'地点',false);await body(trial,'next first native write','sayweave地点');
   channel.sentence(1,'地点在上海',true);await body(trial,'next continued native write','sayweave地点在上海');
   await finishTask(trial,'next finish');await body(trial,'after next finish','sayweave地点在上海');
   trial.learningOverview=await invoke(main,'language_overview',{after:null});trial.passed=true;
   }catch(error){
    trial.error=String(error);evidence.firstError??=String(error);failures.push(trial.error);
    writeFileSync(path.join(directory,'restart-'+(enabled?'on':'off')+'-first-error.json'),JSON.stringify(trial,null,2));
    const cleanupStep=async(label,operation)=>{try{await operation();}catch(error){trial.cleanupErrors.push({label,error:String(error)});}};
    await cleanupStep('events',async()=>{trial.eventsBeforeCleanup=await target.evaluate(()=>window.continuityEvents);});
    await cleanupStep('body before cleanup',async()=>{trial.bodyBeforeCleanup=await settled();});
    await cleanupStep('screenshot',()=>target.screenshot({path:path.join(directory,'restart-'+(enabled?'on':'off')+'-first-error.png')}));
    if(activeTask){
     await cleanupStep('cancel and settle',async()=>{
     await invoke(main,'dictation_control',{taskId:activeTask,action:'cancel'});
     await expect.poll(async()=>{const s=await invoke(main,'dictation_snapshot');return s.taskId===activeTask&&s.delivery!==null&&!['preparing','recording','editing','resuming','recognizing','delivering'].includes(s.presentation);},{timeout:10000}).toBe(true);
     activeTask=null;
     });
    }
    await cleanupStep('body after cancel',async()=>{const sample=await settled();if(activeTask)trial.bodyAfterCancelRequest=sample;else trial.bodyAfterCleanup=sample;});
    writeFileSync(path.join(directory,'restart-'+(enabled?'on':'off')+'-first-error.json'),JSON.stringify(trial,null,2));
    if(trial.cleanupErrors.length){
     if(!enabled)evidence.trials.push({learning:true,status:'notRunDueToCleanup'});
     break;
    }
   }
  }
  if(failures.length)throw Error('Same-main-process restart failed: '+failures.join('; '));
  evidence.inputEvents=await target.evaluate(()=>window.continuityEvents);
  evidence.inputPassed=true;
 }catch(error){
  scenarioError=error;evidence.firstError??=String(error);
 }finally{
  if(target&&!target.isClosed()){
   evidence.inputEvents=await target.evaluate(()=>window.continuityEvents??[]).catch(()=>[]);
   evidence.bodyBeforeCleanup=await target.evaluate(()=>({text:window.fixtureEditor?.read(),state:window.fixtureEditor?.inspect()})).catch(()=>null);
   await target.screenshot({path:path.join(directory,'restart-body-before-cleanup.png')}).catch(()=>{});
  }
  writeFileSync(path.join(directory,'restart-before-cleanup.json'),JSON.stringify(evidence,null,2));
  let cancelled=false;
  if(activeTask){
   try{
    await invoke(main,'dictation_control',{taskId:activeTask,action:'cancel'});
    await expect.poll(async()=>{const s=await invoke(main,'dictation_snapshot');return s.taskId===activeTask&&s.delivery!==null&&!['preparing','recording','editing','resuming','recognizing','delivering'].includes(s.presentation);},{timeout:10000}).toBe(true);
    cancelled=true;
   }catch(error){evidence.cleanupErrors.push(String(error));}
  }else cancelled=true;
  evidence.cleanupSettled=cancelled;
  if(target&&!target.isClosed()){
   const final=await target.evaluate(async()=>{await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));return {text:window.fixtureEditor?.read(),state:window.fixtureEditor?.inspect()};}).catch(()=>null);
   if(cancelled)evidence.bodyAfterCleanup=final;else evidence.bodyAfterCancelRequest=final;
  }
  try{await helper.close();}catch(error){evidence.cleanupErrors.push(String(error));}
  try{await targetBrowser.close();}catch(error){evidence.cleanupErrors.push(String(error));}
  app.stderr.off('data',logListener);
 }
 if(scenarioError)throw scenarioError;
 if(evidence.cleanupErrors.length)throw Error('Main-process fixture cleanup failed: '+evidence.cleanupErrors.join('; '));
 evidence.passed=true;
}
async function tabStartupScenario(main,base){
 const helper=await ownedEditor(),targetBrowser=await chromium.launch({headless:false,args:['--force-renderer-accessibility']});
 const evidence={applicationProcess:app.pid,trials:[],lifecycle:[],cleanupErrors:[]};report.tabStartup=evidence;
 evidence.coverage={ordinaryCore:true,acceptanceOnlyFirstWriteBarrier:false,preparingCancellationScope:'readiness'};
 const listener=chunk=>evidence.lifecycle.push({at:Date.now(),lines:chunk.toString('utf8')});app.stderr.on('data',listener);
 const session=await targetBrowser.newBrowserCDPSession();
 const editorPid=(await session.send('SystemInfo.getProcessInfo')).processInfo.find(p=>p.type==='browser').id;
 await session.detach();evidence.editorProcess=editorPid;evidence.browserVersion=targetBrowser.version();
 let target,held=false,activeTask;
 const keyEvent=async down=>{
  const result=await helper.command({action:'fixture-key',process:editorPid,key:'AltRight',down});
  if(!result.keyResult?.accepted)throw Error('Tab startup key refused');held=down;
 };
 const tap=async()=>{await keyEvent(true);await keyEvent(false);};
 const failures=[];
 try{
  await invoke(main,'model_save',{request:{id:null,revision:null,expectation:null,input:{name:'首笔交接受控整理',protocol:'openai-chat',baseUrl:base+'/v1',modelId:'polish-fixture',usages:['text']},key:{action:'replace',value:key}}});
  expect(await invoke(main,'text_service_status')).toMatchObject({installed:true,enabled:true,updateAvailable:false,error:null});
  const bundle=await buildFixture({configFile:false,logLevel:'error',define:{'process.env.NODE_ENV':JSON.stringify('production')},build:{write:false,lib:{entry:path.join(root,'tests/fixtures/input-target.ts'),name:'InputFixture',formats:['iife']}}});
  const repetitions=Number(process.env.SAYWEAVE_TAB_STARTUP_REPETITIONS??1);
  if(!Number.isInteger(repetitions)||repetitions<1||repetitions>5)throw Error('Invalid Tab startup repetition count');
  for(const shortcutMode of Array.from({length:repetitions},()=>['toggle','hold']).flat()){
   const trial={shortcutMode,bodies:[],events:[],passed:false,cleanupErrors:[]};evidence.trials.push(trial);
   const keys=await invoke(main,'shortcut_status');await invoke(main,'shortcut_save',{config:{...keys.saved,binding:'AltRight',mode:shortcutMode==='hold'?'hold':'toggle'}});
   target=await targetBrowser.newPage();
   await target.setContent('<title>Sayweave 首笔交接验证</title><input id="input"><textarea id="area"></textarea><div id="editable" contenteditable="true"></div>');
   await target.addScriptTag({content:(Array.isArray(bundle)?bundle[0]:bundle).output[0].code});
   await target.evaluate(()=>{
    window.fixtureEditor.set('',0,0);window.fixturePrediction('示例建议 · 按 Tab 接受');window.startupEvents=[];
    for(const type of ['keydown','keyup','compositionstart','compositionupdate','compositionend','beforeinput','input'])document.addEventListener(type,event=>{
     if(!event.target.closest?.('#prosemirror'))return;
     const record=phase=>window.startupEvents.push({type,phase,key:event.key,code:event.code,keyCode:event.keyCode,data:event.data,inputType:event.inputType,at:Date.now(),body:window.fixtureEditor.read(),state:window.fixtureEditor.inspect()});
     record('capture');queueMicrotask(()=>record('microtask'));setTimeout(()=>record('after-task'),0);
    },true);
   });
   const body=async(label,expected)=>{
    const deadline=Date.now()+6000;let text;
    do{
     const sample=await target.evaluate(async()=>{await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));return {text:window.fixtureEditor.read(),state:window.fixtureEditor.inspect(),eventCount:window.startupEvents.length};});
     trial.bodies.push({label,at:Date.now(),...sample});text=sample.text;
     if(text===expected)return sample;await delay(40);
    }while(Date.now()<deadline);
    trial.firstDivergence={label,expected,actual:text};throw Error(label+': actual body differs');
   };
   try{
    await target.bringToFront();await target.locator('#prosemirror .ProseMirror').focus();
    expect((await helper.command({action:'focus-process',process:editorPid})).foregroundProcess).toBe(editorPid);
    trial.initial=await body('true empty field','');expect(trial.initial.state.html).toContain('data-owned-prediction');
    mode='learning-restart-tab-'+shortcutMode+'-next';
    const prior=(await invoke(main,'dictation_snapshot')).taskId;
    if(shortcutMode==='hold')await keyEvent(true);else await tap();
    await expect.poll(async()=>{const s=await invoke(main,'dictation_snapshot');if(s.taskId!==prior)activeTask=s.taskId;trial.startSnapshot=s;return Boolean(activeTask&&s.inputRoute==='live'&&s.presentation==='recording');},{timeout:10000}).toBe(true);
    trial.taskId=activeTask;trial.startKeyReleased=shortcutMode==='toggle'&&!held;
    await expect.poll(()=>restartChannels.at(-1)?.connection.started===true,{timeout:10000}).toBe(true);
    let channel=restartChannels.at(-1);trial.connectionIndex=restartChannels.length-1;
    for(const [revision,text]of [[1,'周'],[2,'舟三开会']]){
     channel.sentence(1,text,false);await body('stream revision '+revision,text);
    }
    channel.sentence(2,'地点在上海',false);await body('next segment','舟三开会地点在上海');
    channel.sentence(1,'舟三开会',true);channel.sentence(2,'地点在上海',true);
    if(shortcutMode==='hold')await keyEvent(false);else await tap();
    await waitPhase(main,'completed');await body('final whole-block processing','周三开会地点在上海');
    trial.finalSnapshot=await invoke(main,'dictation_snapshot');
    expect(trial.finalSnapshot.error).toBeNull();expect(trial.finalSnapshot.inputRoute).toBe('live');
    expect(app.pid).toBe(evidence.applicationProcess);trial.passed=true;activeTask=null;
   }catch(error){trial.error=String(error);failures.push(shortcutMode+': '+error);}
   finally{
    trial.events=await target.evaluate(()=>window.startupEvents).catch(()=>[]);
    trial.finalBody=await target.evaluate(()=>window.fixtureEditor.read()).catch(()=>null);
    if(held)await keyEvent(false).catch(error=>trial.cleanupErrors.push(String(error)));
    if(activeTask)try{
     const cancelledTask=activeTask;
     await invoke(main,'dictation_control',{taskId:cancelledTask,action:'cancel'});
     await expect.poll(async()=>{const state=await invoke(main,'dictation_snapshot');return state.taskId===cancelledTask&&state.delivery!==null&&!['preparing','recording','editing','resuming','recognizing','delivering'].includes(state.presentation);},{timeout:10000}).toBe(true);
     trial.cleanupSettled=true;
    }catch(error){trial.cleanupErrors.push(String(error));}
    activeTask=null;await delay(300);await target.close();target=null;
   }
  }
 }finally{
  app.stderr.off('data',listener);await targetBrowser.close().catch(error=>evidence.cleanupErrors.push(String(error)));await helper.close();
 }
 if(evidence.cleanupErrors.length||evidence.trials.some(t=>t.cleanupErrors.length))throw Error('Tab startup cleanup failed');
 if(failures.length)throw Error(failures.join('; '));
 evidence.passed=true;
}

async function readinessScenario(page){
 const helper=await ownedEditor(),overlay=await overlayPage();
 report.readiness={passed:false,trials:[],claims:{realDevice:true,controlledASR:true,actualSpokenFirstWord:false}};
 let activeTask,heldKey=false;
 const key=async down=>{const result=await helper.command({action:'fixture-key',process:helper.pid,key:'AltRight',down});expect(result.keyResult?.accepted).toBe(true);heldKey=down;};
 await overlay.evaluate(()=>{
  window.readinessFrames=[];window.readinessSampling=true;
  const tick=()=>{if(!window.readinessSampling)return;const node=document.querySelector('[data-phase]');if(node)window.readinessFrames.push({at:Date.now(),phase:node.getAttribute('data-phase'),width:node.getBoundingClientRect().width});requestAnimationFrame(tick);};requestAnimationFrame(tick);
 });
 try{
  for(const name of ['slow-start','cancel-before-ready','release-hold-before-ready','next-start']){
   const trial={name,events:[],passed:false};report.readiness.trials.push(trial);
   const shortcuts=await invoke(page,'shortcut_status');
   await invoke(page,'shortcut_save',{config:{...shortcuts.saved,binding:'AltRight',mode:name==='release-hold-before-ready'?'hold':'toggle'}});
   await helper.command({action:'focus',text:'准备｜',start:3,end:3});
   mode='readiness-'+name;const count=readinessChannels.length;
   trial.startedAt=Date.now();
   if(name==='release-hold-before-ready'){
    const previous=(await invoke(page,'dictation_snapshot')).taskId;await key(true);
    await expect.poll(async()=>{const snapshot=await invoke(page,'dictation_snapshot');if(snapshot.taskId&&snapshot.taskId!==previous)activeTask=snapshot.taskId;return Boolean(activeTask);},{timeout:8000}).toBe(true);
   }else activeTask=(await invoke(page,'dictation_start')).taskId;
   trial.taskId=activeTask;
   await expect.poll(()=>readinessChannels.length,{timeout:8000}).toBe(count+1);
   const channel=readinessChannels.at(-1);trial.receivedAt=channel.receivedAt;
   const checkHeldFrames=async()=>{
    const until=Date.now();const frames=await overlay.evaluate(({from,until})=>window.readinessFrames.filter(frame=>frame.at>=from&&frame.at<=until),{from:trial.startedAt,until});
    trial.heldVisual={from:trial.startedAt,until,count:frames.length,premature:frames.filter(frame=>frame.phase==='recording')};
    expect(frames.length).toBeGreaterThan(0);expect(trial.heldVisual.premature,'no transient expanded frame while service/capture is unready').toEqual([]);
   };
   const sample=async label=>{
    // Read the visual state first: a subsequent Recording snapshot is allowed,
    // but an already expanded UI while capture still Connecting is not.
    const visual=await overlay.evaluate(()=>{const node=document.querySelector('[data-phase]');return {at:Date.now(),phase:node?.getAttribute('data-phase')??'absent',width:node?.getBoundingClientRect().width??0};});
    const speech=await invoke(page,'speech_snapshot'),dictation=await invoke(page,'dictation_snapshot');
    const value={label,visual,speech:{taskId:speech.taskId,phase:speech.phase,sequence:speech.sequence,lifecycle:speech.lifecycle},dictation:{taskId:dictation.taskId,presentation:dictation.presentation,sequence:dictation.sequence},frames:channel.connection.frames};trial.events.push(value);return value;
   };
   const held=await sample('ASR task-started held');
   expect(channel.connection.started).toBe(false);expect(held.speech.taskId).toBe(activeTask);expect(held.speech.phase).toBe('connecting');
   expect(held.dictation.presentation,'no recording layout before ASR/capture ready').toBe('preparing');
   expect(held.visual.phase).toBe('connecting');
   if(name==='cancel-before-ready'||name==='release-hold-before-ready'){
    if(heldKey){await key(false);trial.cancelInput='actual AltRight release';}
    else{await invoke(page,'dictation_control',{taskId:activeTask,action:'cancel'});trial.cancelInput='explicit cancel';}
    await expect.poll(async()=>(await invoke(page,'dictation_snapshot')).presentation,{timeout:8000}).toBe('idle');
    const cancelled=activeTask;await checkHeldFrames();trial.lateRelease=channel.release();
    const after=await sample('late service release after cancel');
    expect(after.dictation.taskId).toBe(cancelled);expect(after.dictation.presentation).toBe('idle');expect(after.visual.phase).not.toBe('recording');expect(channel.connection.frames).toBe(0);
    trial.passed=true;activeTask=null;continue;
   }
   if(name==='slow-start'){
    // Controlled fixture delay only. Production contains no timing workaround.
    await new Promise(resolve=>setTimeout(resolve,Math.max(0,3000-(Date.now()-channel.receivedAt))));
    const blocked=await sample('still held at three seconds');
    expect(blocked.dictation.presentation).toBe('preparing');expect(blocked.visual.phase).toBe('connecting');expect(channel.connection.frames).toBe(0);
   }
   await checkHeldFrames();trial.releaseDelivery=channel.release();trial.releasedAt=channel.releasedAt;expect(trial.releaseDelivery.sent).toBe(true);
   await expect.poll(async()=>{
    const s=await sample('released until capture ready');
    if(s.visual.phase==='recording')expect(s.speech.phase,'expanded recording must follow real capture readiness').toBe('recording');
    return s.visual.phase;
   },{timeout:8000,intervals:[10,20,50]}).toBe('recording');
   await expect.poll(()=>channel.connection.frames,{timeout:5000}).toBeGreaterThanOrEqual(8);
   await expect.poll(async()=>(await helper.command({action:'read'})).text,{timeout:5000}).toBe('准备｜正在识别');
   await invoke(page,'dictation_control',{taskId:activeTask,action:'finish'});await waitPhase(page,'completed');
   await expect.poll(async()=>{const s=await invoke(page,'dictation_snapshot');return s.taskId===activeTask&&s.stage==='result'&&s.delivery!==null;},{timeout:8000}).toBe(true);
   trial.actualBody=(await helper.command({action:'read'})).text;expect(trial.actualBody).toBe('准备｜原生受控识别完成。');
   const final=await invoke(page,'dictation_snapshot');trial.delivery=final.delivery;trial.audioFrames=channel.connection.frames;
   if(final.presentationId)await dismiss(page,{taskId:final.taskId,presentationId:final.presentationId});
   activeTask=null;trial.passed=true;
  }
  report.readiness.passed=true;
 }finally{
  if(heldKey)await key(false).catch(()=>{});
  if(activeTask)await invoke(page,'dictation_control',{taskId:activeTask,action:'cancel'}).catch(()=>{});
  for(const channel of readinessChannels)channel.release();
  report.readiness.visualFrames=await overlay.evaluate(()=>{window.readinessSampling=false;return window.readinessFrames;}).catch(()=>[]);
  await helper.close();
 }
}

async function run(){try{
 // A reviewed controller retry can retain exact already-built product bytes.
 const retained=process.env.SAYWEAVE_SPEECH_REUSE_PREPARED;
 if(retained){
  expect(['learning-restart','tab-startup','readiness','translation']).toContain(process.env.SAYWEAVE_SPEECH_ACCEPTANCE_SCOPE);
  const proof=JSON.parse(readFileSync(retained,'utf8').replace(/^\uFEFF/,''));
  expect(execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim()).toBe(proof.source??proof.controllerSource);
  if(proof.sourceSnapshot){
   const entries=proof.sourceSnapshot.files;expect(Array.isArray(entries)&&entries.length>0).toBe(true);
   for(const required of ['src-tauri/src/dictation.rs','src-tauri/src/dictation/presentation.rs','src-tauri/src/speech/mod.rs','src-tauri/src/speech/continuous.rs','src/contracts/dictation.ts','src/app/dictation-feedback.ts','package.json','src-tauri/Cargo.toml'])expect(entries.some(file=>file.path===required)).toBe(true);
   // The controller is not compiled into the retained product. Every other
   // frozen input stays exact, including fixtures and product/build sources.
   for(const file of entries)if(file.path!=='scripts/test-speech-desktop.mjs')expect(createHash('sha256').update(readFileSync(path.join(root,file.path))).digest('hex'),file.path).toBe(file.sha256);
   report.productSourceDigest=proof.sourceSnapshot.digest;
  }else execFileSync('git',['diff','--quiet','HEAD','--','src','src-tauri'],{cwd:root});
  for(const file of proof.files)expect(createHash('sha256').update(readFileSync(file.path)).digest('hex')).toBe(file.sha256);
  for(const required of [exe,proof.fixtureTest,path.join(root,'src-tauri/target/debug/input-fixture.exe'),path.join(root,'src-tauri/resources/fixed-proxy/abi1.dll'),process.env.SAYWEAVE_TSF_PAYLOAD]){
   expect(required).toBeTruthy();expect(proof.files.some(file=>path.resolve(file.path)===path.resolve(required))).toBe(true);
  }
  report.componentPayloads=verifyComponentEmbeddings([exe],{core:process.env.SAYWEAVE_TSF_PAYLOAD,proxy:path.join(root,'src-tauri/resources/fixed-proxy/abi1.dll')});
  report.reusedBuild=proof;
 }else{
  await buildFixture({logLevel:'error'});
  if(!process.env.SAYWEAVE_TSF_PAYLOAD)await command(['build','--manifest-path','src-tauri/text-service/Cargo.toml','--release']);
  const payload=process.env.SAYWEAVE_TSF_PAYLOAD??path.join(root,'src-tauri/text-service/target/release/sayweave_text_service.dll');report.componentSha256=createHash('sha256').update(readFileSync(payload)).digest('hex');
  await command(['build','--manifest-path','src-tauri/Cargo.toml','--release','--features','desktop-acceptance,tauri/custom-protocol','--bin','desktop-acceptance'],{SAYWEAVE_TSF_PAYLOAD:payload});
  await command(['build','--manifest-path','src-tauri/Cargo.toml','--features','desktop-acceptance','--bin','input-fixture']);
 }
 report.targetGroup=process.env.SAYWEAVE_INPUT_TARGET_GROUP??'all';
 await command(['test','--manifest-path','src-tauri/Cargo.toml','--lib','model_desktop_fixture','--','--ignored']);server=createServer({cert:readFileSync(path.join(data,'fixture-ca.pem')),key:readFileSync(path.join(data,'fixture-key.pem'))});server.on('upgrade',websocket);server.on('request',(req,res)=>void modelFixture(req,res).catch(error=>{report.modelError=String(error);res.destroy();}));await new Promise(r=>server.listen(0,'127.0.0.1',r));const base='https://localhost:'+server.address().port;
 let page=await start(1);
 if(process.env.SAYWEAVE_SPEECH_ACCEPTANCE_SCOPE==='translation'){
  await readModels(page);
  await invoke(page,'model_save',{request:{id:null,revision:null,expectation:null,input:{name:'隔离翻译识别',protocol:'qwen-audio-streaming',baseUrl:base,modelId:expectedModel,usages:['speech']},key:{action:'replace',value:key}}});
  report.scope='translation';await translationScenario(page,base);report.passed=true;return;
 }
 await expect(page.getByRole('link',{name:'组件展示'})).toHaveCount(0);await page.getByRole('button',{name:'配置模型',exact:true}).click();const dialog=page.getByRole('dialog');await delay(250);await page.screenshot({path:path.join(directory,'settings-speech-native.png')});await dialog.getByRole('button',{name:'更多设置'}).click();await dialog.getByLabel('名称',{exact:true}).fill('隔离语音验收');await dialog.getByLabel('服务地址（URL）',{exact:true}).fill(base);await dialog.getByLabel('API 密钥',{exact:true}).fill(key);await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByText('已保存。')).toBeVisible();await dialog.getByRole('button',{name:'关闭',exact:true}).click();await expect(dialog).toHaveCount(0);
 if(polishOnly){await polishScenario(page,base);await stop(page);if(polishCase==='no-history'){page=await start(2);expect((await invoke(page,'history_browse',{query:{search:'',cursor:null}})).total).toBe(0);await stop(page);report.checks.push('restart after history-disabled cancellation recovers without a stale cancellation intent');}report.passed=true;return;}

 const devices=await invoke(page,'speech_devices');report.microphoneCount=devices.length;expect(devices.some(d=>d.isDefault)).toBe(true);
 report.scope=process.env.SAYWEAVE_SPEECH_ACCEPTANCE_SCOPE??'all';
 if(report.scope==='translation'){await translationScenario(page,base);report.passed=true;return;}
 if(report.scope==='result-only'){await resultOnlyScenario(page);report.passed=true;return;}
  if(report.scope==='home-trial'){await homeTrialIsolation(page);report.passed=true;return;}
  if(report.scope==='learning-restart'){await learningRestartScenario(page,base);report.passed=true;return;}
 if(report.scope==='tab-startup'){await tabStartupScenario(page,base);report.passed=true;return;}
 if(report.scope==='readiness'){await readinessScenario(page);report.passed=true;return;}
 if(report.scope!=='all')throw Error('Unknown speech acceptance scope');
 const began=performance.now();await page.getByRole('button',{name:'开始录音',exact:true}).click();const live=await waitPhase(page,'recording');expect(live.source).toBe('trial');await expect(page.getByRole('region',{name:'识别文字',exact:true})).toContainText('正在识别');report.firstFixtureTextMs=Math.round(performance.now()-began);
 const nativeBefore=await processMetrics(),sampleStarted=performance.now();
 report.frames=await page.evaluate(()=>new Promise(resolve=>{const frames=[];let previous=performance.now();const end=previous+1000;function tick(now){frames.push(now-previous);previous=now;if(now<end)requestAnimationFrame(tick);else{frames.sort((a,b)=>a-b);resolve({count:frames.length,p95Ms:frames[Math.floor(frames.length*.95)]});}}requestAnimationFrame(tick);}));
 const nativeAfter=await processMetrics(),sampleMs=performance.now()-sampleStarted;report.nativeMetrics={sampleMs:Math.round(sampleMs),cpuMilliseconds:Math.round((nativeAfter.cpuSeconds-nativeBefore.cpuSeconds)*1000),cpuCorePercent:Math.round((nativeAfter.cpuSeconds-nativeBefore.cpuSeconds)/sampleMs*1000000)/10,workingSetBytes:nativeAfter.workingSetBytes,privateBytes:nativeAfter.privateBytes};
 const cdp=await page.context().newCDPSession(page);await cdp.send('Performance.enable');report.rendererMetrics=(await cdp.send('Performance.getMetrics')).metrics.filter(m=>['JSHeapUsedSize','JSHeapTotalSize','TaskDuration','LayoutDuration','RecalcStyleDuration'].includes(m.name));await cdp.detach();await delay(500);
 let overlay=await overlayPage();expect(overlay).toBeTruthy();expect((await invoke(overlay,'speech_snapshot')).taskId).toBe(live.taskId);
 for(const name of ['history_list','history_browse','model_overview','speech_devices']){expect(await rejected(overlay,name,{query:name==='history_browse'?{search:'',cursor:null}:{search:'',page:1}})).toMatchObject({code:'forbidden'});}
 expect(await rejected(page,'runtime_demo',{action:'start',sessionId:0})).toMatchObject({code:'forbidden'});
 await invoke(page,'runtime_window',{surface:'main',action:'hide'});const ending=performance.now();await invoke(overlay,'speech_control',{taskId:live.taskId,action:'finish'});const completed=await waitPhase(page,'completed');report.finishFixtureWaitMs=Math.round(performance.now()-ending);expect(completed.text).toBe('原生受控识别完成。');expect(completed.saved).toBe(true);expect(completed.saveError).toBe(false);
 await invoke(page,'runtime_window',{surface:'main',action:'show'});let history=await invoke(page,'history_list',{query:{search:'',page:1}});expect(history.total).toBe(1);const record=history.records[0];expect(record.audio.status).toBe('ready');expect(record.audio.bytes).toBeGreaterThan(16000);expect(JSON.stringify(record)).not.toContain(key);expect(JSON.stringify(record)).not.toContain('fileKey');
 const feed=await invoke(page,'history_browse',{query:{search:'',cursor:null}});expect(feed.total).toBe(1);expect(feed.records[0].preview).toBe('原生受控识别完成。');expect(feed.records[0].displayAttempt).toBe(record.currentAttempt);expect(feed.records[0].attempts).toBeUndefined();expect(JSON.stringify(feed)).not.toContain(key);expect(JSON.stringify(feed)).not.toContain('fileKey');
 await invoke(page,'history_play',{resourceId:record.audio.id,action:'play',positionMs:null});await delay(100);await invoke(page,'history_play',{resourceId:null,action:'pause',positionMs:null});await invoke(page,'history_play',{resourceId:null,action:'seek',positionMs:100});await invoke(page,'history_play',{resourceId:null,action:'resume',positionMs:null});await invoke(page,'history_play',{resourceId:null,action:'stop',positionMs:null});
 report.checks.push('actual default microphone, native TLS stream, dual-window state, demo/permissions isolated, finalized WAV and playback');
 await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByText('1 条',{exact:true})).toBeVisible();const row=page.getByRole('listitem').first();await row.hover();const playButton=row.getByRole('button',{name:'播放录音',exact:true});await playButton.focus();await playButton.click();await expect(row.getByRole('region',{name:'历史识别文字'})).toContainText('原生受控识别完成。');await expect(page.getByRole('dialog')).toHaveCount(0);await row.getByRole('button',{name:'停止播放'}).click();await page.screenshot({path:path.join(directory,'history.png')});

 await page.getByRole('button',{name:'保存设置',exact:true}).click();await page.getByRole('combobox',{name:'录音保留时间'}).selectOption('7');await page.getByRole('link',{name:'首页',exact:true}).click();await expect(page.getByRole('alertdialog',{name:'有未保存的修改'})).toBeVisible();await page.getByRole('button',{name:'继续填写'}).click();await page.getByRole('button',{name:'保存设置',exact:true}).last().click();await expect(page.getByText('已保存，下一次录音生效。')).toBeVisible();await page.getByRole('button',{name:'保存设置',exact:true}).first().click();
 report.checks.push('main-only bounded history summaries, inline native playback and retention draft protection');report.viewport=await page.evaluate(()=>({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,overflow:document.documentElement.scrollWidth>innerWidth}));expect(report.viewport.overflow).toBe(false);await stop(page);page=await start(2);
 overlay=await overlayPage();history=await invoke(page,'history_list',{query:{search:'',page:1}});expect(history.records[0].audio.status).toBe('ready');expect(history.records[0].attempts[0].text).toBe('原生受控识别完成。');const view=await readModels(page),config=view.configs[0];
 const retryArgs={id:record.id,configId:config.id,configRevision:config.revision};mode='pending';const retry=await invoke(page,'history_retry',retryArgs);await waitPhase(page,'processing');await delay(100);await invoke(page,'speech_control',{taskId:retry.taskId,action:'cancel'});await waitPhase(page,'cancelled');expect((await invoke(page,'history_detail',{id:record.id})).currentAttempt).toBe(record.currentAttempt);
 mode='failure';await invoke(page,'history_retry',retryArgs);await waitPhase(page,'failed');const failedRetry=await invoke(page,'speech_snapshot');expect(failedRetry.canRetry).toBe(true);mode='normal';const recovered=await invoke(page,'speech_retry_current',{taskId:failedRetry.taskId});expect(recovered.recordId).toBe(record.id);await waitPhase(page,'completed');const updated=await invoke(page,'history_detail',{id:record.id});expect(updated.attempts).toHaveLength(4);const retryFeed=await invoke(page,'history_browse',{query:{search:'',cursor:null}});expect(retryFeed.records.find(r=>r.id===record.id).displayAttempt).toBe(updated.currentAttempt);expect(updated.currentAttempt).not.toBe(record.currentAttempt);expect((await invoke(page,'history_usage')).count).toBe(1);
 mode='pending';const cancelled=await invoke(page,'speech_start');await waitPhase(page,'recording');await delay(200);await invoke(page,'speech_control',{taskId:cancelled.taskId,action:'finish'});await waitPhase(page,'processing');const layout=await invoke(page,'runtime_snapshot');await delay(700);expect((await invoke(page,'runtime_snapshot')).revision).toBe(layout.revision);expect(await overlay.evaluate(()=>({width:innerWidth,height:innerHeight}))).toEqual({width:160,height:88});await invoke(page,'speech_control',{taskId:cancelled.taskId,action:'cancel'});await waitPhase(page,'cancelled');expect((await invoke(page,'history_list',{query:{search:'',page:1}})).total).toBe(1);
 report.checks.push('process restart preserves audio/text/vault; cancelled retry keeps success; successful retry adds version without counting again; processing cancel discards new recording');
 mode='failure';await invoke(page,'speech_start');await waitPhase(page,'failed');const failed=await invoke(page,'speech_snapshot');expect(failed.error).toBe('service');const failedHistory=await invoke(page,'history_list',{query:{search:'',page:1}});expect(failedHistory.total).toBe(2);expect(failedHistory.records.some(r=>r.status==='failed')).toBe(true);
 // The most recent captured PCM belongs to B, but the displayed failed retry is A.
 await invoke(page,'history_retry',retryArgs);const displayedRetry=await waitPhase(page,'failed');mode='normal';const bound=await invoke(page,'speech_retry_current',{taskId:displayedRetry.taskId});expect(bound.recordId).toBe(record.id);await waitPhase(page,'completed');expect((await invoke(page,'history_detail',{id:failed.recordId})).attempts).toHaveLength(1);
 for(const scenario of ['auth','empty']){mode=scenario;await invoke(page,'speech_start');if(scenario==='empty'){await waitPhase(page,'recording');await delay(150);const current=await invoke(page,'speech_snapshot');await invoke(page,'speech_control',{taskId:current.taskId,action:'finish'});}await waitPhase(page,'failed');expect((await invoke(page,'speech_snapshot')).error).toBe(scenario==='auth'?'auth':'emptyText');}
 const preferences=await invoke(page,'speech_preferences',{preferences:null});await invoke(page,'speech_preferences',{preferences:{...preferences,historyEnabled:false,audioEnabled:false}});const before=(await invoke(page,'history_list',{query:{search:'',page:1}})).total;mode='normal';const temporary=await invoke(page,'speech_start');await waitPhase(page,'recording');await delay(150);await invoke(page,'speech_control',{taskId:temporary.taskId,action:'finish'});await waitPhase(page,'completed');expect((await invoke(page,'speech_snapshot')).canRetry).toBe(true);const temporaryRetry=await invoke(page,'speech_retry_current',{taskId:temporary.taskId});expect(temporaryRetry.recordId).toBeNull();await waitPhase(page,'completed');expect((await invoke(page,'history_list',{query:{search:'',page:1}})).total).toBe(before);report.checks.push('retry current binds displayed persistent/temporary identity; no-history audio retries only in memory');
 // Change the actual saved record through the model menu and exercise the same native adapter.
 await page.getByRole('link',{name:'首页',exact:true}).click();
 await page.getByRole('button',{name:'配置模型',exact:true}).click();
 const settings=page.getByRole('dialog');await settings.getByRole('button',{name:'编辑',exact:true}).click();
 await settings.getByRole('combobox',{name:'模型',exact:true}).click();await page.getByRole('option',{name:'Qwen Audio 3.0 · 实时识别',exact:true}).click();
 await settings.getByRole('button',{name:'保存',exact:true}).click();await expect(settings.getByText('已保存。')).toBeVisible();await page.keyboard.press('Escape');
 const changed=(await readModels(page)).configs[0];expect(changed.id).toBe(config.id);expect(changed.name).toBe(config.name);expect(changed.usages).toEqual(config.usages);expect(changed.credentialStatus).toBe('present');expect(changed.baseUrl).toBe(base);expect(changed.modelId).toBe('qwen-audio-3.0-asr-flash-streaming');
 expectedModel=changed.modelId;mode='normal';const selected=await invoke(page,'speech_start');await waitPhase(page,'recording');await delay(150);await invoke(page,'speech_control',{taskId:selected.taskId,action:'finish'});await waitPhase(page,'completed');
 expect(report.connections.some(c=>c.model==='qwen-audio-3.1-asr-flash-streaming'&&c.finished)).toBe(true);expect(report.connections.some(c=>c.model==='qwen-audio-3.0-asr-flash-streaming'&&c.finished)).toBe(true);
 await stop(page);page=await start(3);overlay=await overlayPage();const reopened=(await readModels(page)).configs[0];expect(reopened.id).toBe(config.id);expect(reopened.modelId).toBe(expectedModel);expect(reopened.credentialStatus).toBe('present');
 report.checks.push('3.1 and3.0 selected in UI map to run-task model; custom origin/name/key retained;3.0 choice survives restart');
 // Exercise the second protocol through the actual model editor, vault and microphone.
 expectedProtocol='doubao';mode='normal';await page.getByRole('button',{name:'配置模型',exact:true}).click();
 const doubaoSettings=page.getByRole('dialog');await doubaoSettings.getByRole('button',{name:'编辑',exact:true}).click();
 await doubaoSettings.getByRole('combobox',{name:'模型',exact:true}).click();await page.getByRole('option',{name:'豆包 Seed-ASR 2.0 · 实时识别',exact:true}).click();
 await expect(doubaoSettings.getByLabel('API 密钥')).toHaveValue('');await expect(doubaoSettings.getByLabel('API 密钥')).toHaveAttribute('placeholder','粘贴 API 密钥');
 await doubaoSettings.getByRole('button',{name:'更多设置'}).click();await doubaoSettings.getByLabel('服务地址（URL）').fill(base);await doubaoSettings.getByLabel('API 密钥').fill(key);await doubaoSettings.getByRole('button',{name:'保存',exact:true}).click();await expect(doubaoSettings.getByText('已保存。')).toBeVisible();await page.keyboard.press('Escape');
 await page.getByRole('button',{name:'配置模型',exact:true}).click();await page.getByRole('dialog').getByRole('tab',{name:'文字处理'}).click();await delay(250);await page.screenshot({path:path.join(directory,'settings-text-native.png')});await page.keyboard.press('Escape');
 const doubaoConfig=(await readModels(page)).configs[0];expect(doubaoConfig.protocol).toBe('doubao-asr-streaming');expect(doubaoConfig.id).toBe(config.id);expect(doubaoConfig.modelId).toBe('Doubao-Seed-ASR-2.0');
 const prefs=await invoke(page,'speech_preferences',{preferences:null});await invoke(page,'speech_preferences',{preferences:{...prefs,historyEnabled:true,audioEnabled:true}});
 const doubaoTask=await invoke(page,'speech_start');await waitPhase(page,'recording');await expect(page.getByRole('region',{name:'识别文字',exact:true})).toContainText('正在识别');await delay(200);await invoke(page,'speech_control',{taskId:doubaoTask.taskId,action:'finish'});const doubaoDone=await waitPhase(page,'completed');expect(doubaoDone.text).toBe('原生受控识别完成。');expect(doubaoDone.saved).toBe(true);
 const doubaoRecord=await invoke(page,'history_detail',{id:doubaoDone.recordId});expect(doubaoRecord.audio.status).toBe('ready');
 mode='empty-ack';await invoke(page,'history_retry',{id:doubaoRecord.id,configId:doubaoConfig.id,configRevision:doubaoConfig.revision});const acknowledged=await waitPhase(page,'completed');expect(acknowledged.text).toBe('正在识别');expect(acknowledged.saved).toBe(true);
 const beforeMalformed=await invoke(page,'history_detail',{id:doubaoRecord.id});mode='malformed-terminal';await invoke(page,'history_retry',{id:doubaoRecord.id,configId:doubaoConfig.id,configRevision:doubaoConfig.revision});await waitPhase(page,'failed');expect((await invoke(page,'speech_snapshot')).error).toBe('protocol');expect((await invoke(page,'history_detail',{id:doubaoRecord.id})).currentAttempt).toBe(beforeMalformed.currentAttempt);await expect(page.getByRole('group',{name:'识别失败',exact:true})).toHaveCount(0);await expect(page.getByRole('region',{name:'识别文字',exact:true})).toContainText(doubaoDone.text);await expect(overlay.getByRole('group',{name:'识别失败',exact:true})).toHaveCount(1);await expect(page.getByText('演示失败',{exact:true})).toHaveCount(0);
 report.checks.push('negative terminal sentinel accepts final text and empty acknowledgement; malformed terminal stays failed, preserves prior success and uses real recognition labels in overlay while retaining unrelated Home trial');
 mode='resource-denied';const handshakes=report.handshakes.length;await invoke(page,'history_retry',{id:doubaoRecord.id,configId:doubaoConfig.id,configRevision:doubaoConfig.revision});await waitPhase(page,'completed');expect(report.handshakes.slice(handshakes).map(h=>h.resource)).toEqual(['volc.seedasr.sauc.duration','volc.seedasr.sauc.concurrent']);
 mode='auth';const authHandshakes=report.handshakes.length;await invoke(page,'speech_start');await waitPhase(page,'failed');expect((await invoke(page,'speech_snapshot')).error).toBe('auth');expect(report.handshakes.length-authHandshakes).toBe(1);
 mode='pending';const doubaoCancel=await invoke(page,'history_retry',{id:doubaoRecord.id,configId:doubaoConfig.id,configRevision:doubaoConfig.revision});await waitPhase(page,'processing');await invoke(page,'speech_control',{taskId:doubaoCancel.taskId,action:'cancel'});await waitPhase(page,'cancelled');
 report.checks.push('Doubao model UI mapping, cross-service key reset, native binary/gzip stream, partial/final deduplication, WAV persistence and retry; explicit resource-only fallback; auth does not retry; cancellation preserves history');
 await stop(page);page=await start(4);const doubaoReopened=(await readModels(page)).configs[0];expect(doubaoReopened.protocol).toBe('doubao-asr-streaming');expect(doubaoReopened.credentialStatus).toBe('present');expect((await invoke(page,'history_detail',{id:doubaoRecord.id})).audio.status).toBe('ready');
 report.checks.push('Doubao selection, credential and history survive a real process restart');
 mode='normal';
 report.componentStatus=await invoke(page,'text_service_status');
 expect(report.componentStatus,'Installed component must match this build; update through the normal app before testing.').toMatchObject({installed:true,enabled:true,payloadAvailable:true,updateAvailable:false,error:null});
 const keys=await invoke(page,'shortcut_status');expect(keys.active.binding).toBe('AltRight');
 const captureToken=randomUUID();const capturingKeys=await invoke(page,'shortcut_capture',{begin:true,token:captureToken});expect(capturingKeys.captureToken).toBe(captureToken);expect(capturingKeys.capturing).toBe(true);expect(capturingKeys.sequence).toBeGreaterThan(keys.sequence);
 const stoppedCapture=await invoke(page,'shortcut_capture',{begin:false});expect(stoppedCapture.captureToken).toBe(captureToken);expect(stoppedCapture.capturing).toBe(false);expect(stoppedCapture.sequence).toBeGreaterThan(capturingKeys.sequence);
 expect((await rejected(page,'shortcut_capture',{begin:true,token:'invalid'})).code).toBe('invalidShortcut');
 const savedKeys=await invoke(page,'shortcut_save',{config:{...keys.saved,binding:'Control+Shift+F10',mode:'hold'}});expect(savedKeys.active.mode).toBe('hold');
 overlay=await overlayPage();
 for(const command of ['shortcut_status','shortcut_save','history_delivery'])expect((await rejected(overlay,command,command==='history_delivery'?{id:doubaoRecord.id}:command==='shortcut_save'?{config:savedKeys.saved}:{})).code).toBe('forbidden');
 if(process.env.SAYWEAVE_INPUT_TARGET_GROUP!=='browser'){
 const editor=await ownedEditor();try{
  expect((await editor.command({action:'focus',text:'before tail',start:7,end:11})).foreground).toBe(true);
  await invoke(page,'runtime_window',{surface:'overlay',action:'show'});mode='delayed-final';
  const result=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text,{timeout:5000}).toBe('before 正在识别');const before=await editor.command({action:'read'});expect(before.foreground).toBe(true);await invoke(page,'shortcut_capture',{begin:false});await invoke(page,'shortcut_capture',{begin:false});await delay(180);const after=await editor.command({action:'read'});report.externalFocus={beforeProcess:before.foregroundProcess,afterProcess:after.foregroundProcess,applicationProcess:app.pid};const taskId=(await invoke(page,'dictation_snapshot')).taskId;const updates=report.taskTrace.filter(e=>e.line.includes('task_id='+taskId+' ')&&e.line.includes('action=Update applied=true')).length;expect(updates).toBeGreaterThanOrEqual(2);report.streamingInput={beforeFinishActualText:true,acknowledgedUpdatesBeforeFinish:updates,clipboardUsed:false,taskId};expect(after.foreground).toBe(true);},async()=>{report.nativeEndClick=await nativeOverlayClick(overlay,editor,'结束录音');});
  report.externalEdit=result.dictation.delivery;expect(result.dictation.delivery.state).toBe('unconfirmed');expect(result.dictation.delivery.verification).toBe('unavailable');expect((await rejected(page,'dictation_copy',{taskId:'stale-task',presentationId:randomUUID()})).code).toBe('staleSession');expect((await editor.command({action:'read'})).text).toBe('before 原生受控识别完成。');
  expect(result.dictation.presentation).toBe('completed');await expect(overlay.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);await expect.poll(async()=>(await invoke(page,'runtime_snapshot')).windows.overlay,{timeout:2500}).toBe(false);expect((await invoke(page,'dictation_snapshot')).presentation).toBe('idle');mode='normal';
  expect((await rejected(page,'speech_retry_current',{taskId:'stale-displayed-task'})).code).toBe('staleSession');
  expect((await invoke(page,'history_delivery',{id:result.speech.recordId}))[0].state).toBe('unconfirmed');
  expect((await editor.command({action:'read'})).foreground).toBe(true);
  report.overlayInsertion=[];
  mode='unicode';
  for(const sample of [{before:'前面尾部',start:2,end:4},{before:'',start:0,end:0},{before:'a😀b',start:1,end:3}]){
   await editor.command({action:'focus',text:sample.before,start:sample.start,end:sample.end});
   const result=await externalRound(page,async()=>delay(180),async()=>nativeOverlayClick(overlay,editor,'结束录音'));
   const actual=await editor.command({action:'read'});
   expect(actual.text.replace(/\r\n/g,'\n')).toBe(sample.before.slice(0,sample.start)+'中文，🙂\n第二行 e\u0301。'+sample.before.slice(sample.end));
   expect(actual.foreground).toBe(true);expect(result.dictation.delivery.state).toBe('unconfirmed');expect(result.dictation.delivery.verification).toBe('unavailable');
   await expect.poll(async()=>(await invoke(page,'runtime_snapshot')).windows.overlay,{timeout:2500}).toBe(false);
   report.overlayInsertion.push({taskId:result.dictation.taskId,beforeLength:sample.before.length,actualWrite:true,delivery:result.dictation.delivery});
  }
  mode='normal';
  const legacy=await ownedEditor(false);
  try{
   await legacy.command({action:'focus',text:'legacy ',start:7,end:7});
   const legacyResult=await externalRound(page,async()=>{await delay(180);report.plainEditContext=await legacy.command({action:'inspect-context'});});
   expect((await legacy.command({action:'read'})).text,'Plain EDIT compatibility must actually receive its final text: '+JSON.stringify(legacyResult.dictation.delivery)).toBe('legacy 原生受控识别完成。');
   expect(legacyResult.dictation.delivery.submission).toBe('submitted');
   report.plainEditCompatibility=legacyResult.dictation.delivery;
  }catch(error){report.compatibilityFailures??=[];report.compatibilityFailures.push({control:'Win32 EDIT',error:String(error)});}finally{await legacy.close();}
  const other=await ownedEditor();
  try{
   await editor.command({action:'focus',text:'protected',start:9,end:9});
   const away=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('protected正在识别');expect((await other.command({action:'focus'})).foreground).toBe(true);expect((await editor.command({action:'focus'})).foreground).toBe(true);},undefined,true);
   expect(away.dictation.delivery.state).not.toBe('confirmed');
   expect((await editor.command({action:'read'})).text).toBe('protected正在识别');
   expect((await other.command({action:'read'})).text).toBe('');
   await dismiss(page,{taskId:away.dictation.taskId,presentationId:away.dictation.presentationId});
   report.externalSwitchBack=away.dictation.delivery;
  }finally{await other.close();}
  await editor.command({action:'focus',text:'protected',start:9,end:9});
  const mainSwitch=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('protected正在识别');await invoke(page,'runtime_window',{surface:'main',action:'show'});await delay(100);expect((await editor.command({action:'read'})).foregroundProcess).toBe(app.pid);expect((await editor.command({action:'focus'})).foreground).toBe(true);},undefined,true);
  expect(mainSwitch.dictation.delivery.state).not.toBe('confirmed');
  expect((await editor.command({action:'read'})).text).toBe('protected正在识别');
  await dismiss(page,{taskId:mainSwitch.dictation.taskId,presentationId:mainSwitch.dictation.presentationId});report.mainSwitchBack=mainSwitch.dictation.delivery;
  await editor.command({action:'focus',text:'protected',start:9,end:9});
  const caret=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('protected正在识别');report.caretSelection=await editor.command({action:'caret-home'});expect(report.caretSelection.start).toBe(0);expect(report.caretSelection.end).toBe(0);},undefined,true);
  expect(caret.dictation.delivery.state).not.toBe('confirmed');
  expect((await editor.command({action:'read'})).text).toBe('protected正在识别');
  await dismiss(page,{taskId:caret.dictation.taskId,presentationId:caret.dictation.presentationId});report.caretChange=caret.dictation.delivery;
  expect((await editor.command({action:'focus',text:'initial',start:7,end:7})).foreground).toBe(true);
  const changed=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('initial正在识别');await editor.command({action:'focus',text:'user changed',start:0,end:0});},undefined,true);
  expect(changed.dictation.delivery.state).not.toBe('confirmed');await dismiss(page,{taskId:changed.dictation.taskId,presentationId:changed.dictation.presentationId});expect((await editor.command({action:'read'})).text).toBe('user changed');
  report.checks.push('ASR partial enters actual native selection before finish; real Win32 mouse end click updates one range with final Unicode/multiline/emoji and retains foreground; external/main switches preserve handed-off visible text without further writes after switching back; programmatic target edits retain user text; delayed provider tail waits and overlay hides; journal and overlay permissions verified');
 }finally{await editor.close();}
 }
 const targetBrowser=await chromium.launch({headless:false,args:['--force-renderer-accessibility']});
 const focusHelper=await ownedEditor();
 try{const session=await targetBrowser.newBrowserCDPSession();const process=(await session.send('SystemInfo.getProcessInfo')).processInfo.find(p=>p.type==='browser').id;await session.detach();const target=await targetBrowser.newPage();await target.setContent('<input id="input"/><textarea id="area"></textarea><textarea id="area2"></textarea><div id="editable" contenteditable="true"></div><textarea id="readonly" readonly></textarea><input id="password" type="password"/><div id="mixed" contenteditable="true"></div><div id="body" tabindex="0"></div>');
  report.inputCompatibility=[];
  for(const id of ['input','area','editable','readonly','password','mixed','body']){const nonce='fixture-'+randomUUID(),original=id==='mixed'?nonce+' fixed':nonce;await target.locator('#'+id).evaluate((node,nonce)=>{if('value'in node)node.value=nonce;else if(node.id==='mixed')node.innerHTML=nonce+'<span contenteditable="false"> fixed</span>';else node.textContent=nonce;},nonce);await target.bringToFront();await target.locator('#'+id).focus();await target.locator('#'+id).evaluate(node=>{if(node instanceof HTMLInputElement||node instanceof HTMLTextAreaElement)node.setSelectionRange(node.value.length,node.value.length);else{const selection=getSelection(),range=document.createRange();if(node.id==='mixed'){range.setStart(node.firstChild,node.firstChild.textContent.length);range.collapse(true);}else{range.selectNodeContents(node);range.collapse(false);}selection.removeAllRanges();selection.addRange(range);}});
   expect((await focusHelper.command({action:'focus-process',process})).foregroundProcess).toBe(process);await delay(200);
   if(id==='password') {
    const connections=report.connections.length,total=(await invoke(page,'history_browse',{query:{search:'',cursor:null}})).total;
    const failure=await rejected(page,'dictation_start');const expectedError='targetPassword';if(failure.code!==expectedError){report.compatibilityFailures??=[];report.compatibilityFailures.push({control:id,error:failure.code,expectedError});}
    const rejectedView=await invoke(page,'dictation_snapshot');expect(rejectedView.admission).toBe('rejected');expect(rejectedView.inputRoute).toBe('live');expect(rejectedView.delivery).toBeNull();
    expect(report.connections.length).toBe(connections);expect((await invoke(page,'history_browse',{query:{search:'',cursor:null}})).total).toBe(total);expect((await invoke(page,'speech_snapshot')).recordId).toBeNull();
    expect(await target.locator('#'+id).inputValue()).toBe(original);await dismiss(page,{taskId:rejectedView.taskId,presentationId:rejectedView.presentationId});report.inputCompatibility.push({app:'Chromium',control:id,admission:'rejected',error:failure.code,noRecording:true});continue;
   }
   const result=await externalRound(page,async()=>delay(150),['input','area','editable','mixed'].includes(id)?async()=>nativeOverlayClick(overlay,focusHelper,'结束录音'):undefined);const actual=await target.locator('#'+id).evaluate(node=>'value'in node?node.value:node.textContent);
   const expected=id==='mixed'?nonce+'原生受控识别完成。 fixed':original+'原生受控识别完成。';if(['readonly','body'].includes(id)){expect(result.dictation.inputRoute).toBe('resultOnly');expect(result.dictation.presentation).toBe('result');expect(result.dictation.delivery.error).toBeNull();expect(result.dictation.delivery.submission).toBe('notSubmitted');expect(actual).toBe(original);}else{expect(actual,'Writable '+id+' must actually receive text: '+JSON.stringify(result.dictation.delivery)).toBe(expected);expect(result.dictation.delivery.submission).toBe('submitted');}
   if(result.dictation.delivery.state==='confirmed'||result.dictation.delivery.state==='unconfirmed')expect(result.dictation.presentation).not.toBe('result');
   if(result.dictation.presentation==='result'){await expect(overlay.getByRole('button',{name:'复制',exact:true})).toBeVisible();await expect(overlay.getByRole('button',{name:'关闭',exact:true})).toBeVisible();}
   if(result.dictation.presentationId)await dismiss(page,{taskId:result.dictation.taskId,presentationId:result.dictation.presentationId});
   report.inputCompatibility.push({app:'Chromium',control:id,presentation:result.dictation.presentation,delivery:result.dictation.delivery.state,error:result.dictation.delivery.error,observedWrite:actual===expected});
  }
 // Same-window changes must protect both same-type and different-type controls.
  report.sameWindowControls=[];
  for(const next of ['input','area2']){
   await target.bringToFront();await target.locator('#area').evaluate(node=>{node.removeAttribute('maxlength');node.value='original';node.focus();node.setSelectionRange(8,8);});
   await target.locator('#'+next).evaluate(node=>{node.value='other-control';});
   await focusHelper.command({action:'focus-process',process});await delay(100);
   const switched=await externalRound(page,async()=>{await expect.poll(()=>target.locator('#area').inputValue()).toBe('original正在识别');await target.locator('#'+next).focus();await delay(100);},undefined,true);
   expect(await target.locator('#area').inputValue()).toBe('original正在识别');expect(await target.locator('#'+next).inputValue()).toBe('other-control');
   expect(switched.dictation.delivery.state).not.toBe('confirmed');report.sameWindowControls.push({next,trigger:'DOM focus; physical keys separately validated',originalPreserved:true,newControlUntouched:true,delivery:switched.dictation.delivery});
   await dismiss(page,{taskId:switched.dictation.taskId,presentationId:switched.dictation.presentationId});
  }
 // Compatibility observation: script-only caret movement may be invisible to TSF.
 // This accepted limitation is reported separately; physical editing remains a core guard.
  await target.bringToFront();await target.locator('#area').evaluate(node=>{node.value='a'.repeat(6000);node.focus();node.setSelectionRange(3000,3000);});
  await focusHelper.command({action:'focus-process',process});await delay(100);
  const partialLong='a'.repeat(3000)+'正在识别'+'a'.repeat(3000);
  const moved=await externalRound(page,async()=>{await expect.poll(()=>target.locator('#area').inputValue()).toBe(partialLong);const selection=await target.locator('#area').evaluate(node=>{node.setSelectionRange(3500,3500);return [node.selectionStart,node.selectionEnd];});expect(selection).toEqual([3500,3500]);await delay(100);},undefined,true);
  const movedText=await target.locator('#area').inputValue();report.programmaticCaret={selection:[3500,3500],partialRetained:movedText===partialLong,delivery:moved.dictation.delivery};expect(moved.dictation.delivery.state).not.toBe('confirmed');if(movedText!==partialLong){report.acceptedLimitations??=[];report.acceptedLimitations.push({control:'long textarea programmatic selection',observed:'late draft update after DOM caret move',protectionPassed:false,scope:'script-only; physical-edit protection remains required'});}
  await dismiss(page,{taskId:moved.dictation.taskId,presentationId:moved.dictation.presentationId});
  mode='single-a';await target.locator('#area').evaluate(node=>{node.value='a'.repeat(3000);node.maxLength=3000;node.focus();node.setSelectionRange(3000,3000);});await focusHelper.command({action:'focus-process',process});await delay(100);
  const refused=await externalRound(page,async()=>delay(100),undefined,true);const refusedText=await target.locator('#area').inputValue();report.maxLength={classification:'target-constraint',unchanged:refusedText==='a'.repeat(3000),delivery:refused.dictation.delivery};expect(refusedText).toBe('a'.repeat(3000));expect(refused.dictation.delivery.verification).toBe('unavailable');expect(refused.speech.text).toBe('a');if(refusedText!=='a'.repeat(3000)||refused.dictation.delivery.state==='confirmed'){report.compatibilityFailures??=[];report.compatibilityFailures.push({control:'maxlength textarea',error:'composition constraint or confirmation mismatch'});}
  report.inputProtection={moved:moved.dictation.delivery,refused:refused.dictation.delivery};
  await dismiss(page,{taskId:refused.dictation.taskId,presentationId:refused.dictation.presentationId});mode='normal';
 const bundle=await buildFixture({configFile:false,logLevel:'error',build:{write:false,lib:{entry:path.join(root,'tests/fixtures/input-target.ts'),name:'InputFixture',formats:['iife']}}});
  await target.addScriptTag({content:(Array.isArray(bundle)?bundle[0]:bundle).output[0].code});
  for(const sample of [{text:'前面尾部',start:2,end:4},{text:'很长的文档。'.repeat(8000),start:24000,end:24000}]){
   await target.bringToFront();await target.evaluate(sample=>window.fixtureEditor.set(sample.text,sample.start,sample.end),sample);
   expect((await focusHelper.command({action:'focus-process',process})).foregroundProcess).toBe(process);await delay(100);
   const result=await externalRound(page,async()=>delay(100),async()=>nativeOverlayClick(overlay,focusHelper,'结束录音'));const actual=await target.evaluate(()=>window.fixtureEditor.read());
   expect(actual,'ProseMirror must receive text: '+JSON.stringify(result.dictation.delivery)).toBe(sample.text.slice(0,sample.start)+'原生受控识别完成。'+sample.text.slice(sample.end));
   expect(result.dictation.delivery.submission).toBe('submitted');
   report.inputCompatibility.push({app:'Chromium',control:'ProseMirror',length:sample.text.length,delivery:result.dictation.delivery});
   if(result.dictation.presentationId)await dismiss(page,{taskId:result.dictation.taskId,presentationId:result.dictation.presentationId});
  }
  mode='unicode';
  for(const sample of [
   ...['area','editable','ProseMirror'].map(control=>({control,before:'前面尾部',start:2,end:4})),
   {control:'area',before:'',start:0,end:0},
   {control:'ProseMirror',before:'',start:0,end:0},
   {control:'ProseMirror',before:'尾部',start:0,end:0},
  ]){
   const {control}=sample;
   await target.bringToFront();
   if(control==='ProseMirror')await target.evaluate(sample=>window.fixtureEditor.set(sample.before,sample.start,sample.end),sample);
   else await target.locator('#'+control).evaluate((node,sample)=>{if('value'in node){node.removeAttribute('maxlength');node.value=sample.before;node.focus();node.setSelectionRange(sample.start,sample.end);}else{node.textContent=sample.before;node.focus();const range=document.createRange();range.setStart(node.firstChild,sample.start);range.setEnd(node.firstChild,sample.end);const selected=getSelection();selected.removeAllRanges();selected.addRange(range);}},sample);
   await focusHelper.command({action:'focus-process',process});await delay(150);
   const result=await externalRound(page,async()=>delay(100));
   const actual=control==='ProseMirror'?await target.evaluate(()=>window.fixtureEditor.read()):await target.locator('#'+control).evaluate(node=>'value'in node?node.value:node.innerText);
   let exactText=true;try {
   expect(actual.replace(/\r\n/g,'\n')).toBe(sample.before.slice(0,sample.start)+'中文，🙂\n第二行 e\u0301。'+sample.before.slice(sample.end));
   } catch(error) {exactText=false;report.compatibilityFailures??=[];report.compatibilityFailures.push({control,case:'unicode-multiline',beforeLength:sample.before.length,start:sample.start,error:String(error)});}

   expect(result.dictation.delivery.state,'Commit has no provider completion fence: '+JSON.stringify(result.dictation.delivery)).toBe('unconfirmed');expect(result.dictation.delivery.verification).toBe('unavailable');
   report.inputCompatibility.push({app:'Chromium',control,case:'unicode-multiline',exactText,start:sample.start,beforeLength:sample.before.length,delivery:result.dictation.delivery});
   if(result.dictation.presentationId)await dismiss(page,{taskId:result.dictation.taskId,presentationId:result.dictation.presentationId});
  }
  mode='normal';
  const shortcut=(await invoke(page,'shortcut_status')).saved;
  await invoke(page,'shortcut_save',{config:{...shortcut,binding:'AltRight',mode:'hold'}});
  const keyEvent=(key,down)=>focusHelper.command({action:'fixture-key',process,key,down});
  try {
   for(const control of ['area','ProseMirror']) {
    await target.bringToFront();
    if(control==='area'){await target.locator('#area').fill('abc');await target.locator('#area').focus();}
    else await target.evaluate(()=>window.fixtureEditor.set('abc',3,3));
    expect((await focusHelper.command({action:'focus-process',process})).foregroundProcess).toBe(process);
    const read=()=>control==='area'?target.locator('#area').inputValue():target.evaluate(()=>window.fixtureEditor.read());
    const result=await externalRound(page,async(start)=>{
     await expect.poll(read).toBe('abc正在识别');
     await keyEvent('Backspace',true);
     // A held edit key must not acquire a new range halfway through editing.
     await delay(700);
     expect(report.taskTrace.some(e=>e.line.includes('task_id='+start.taskId+' ')&&e.line.includes('turn=2')&&e.line.includes('action=Begin'))).toBe(false);
     await keyEvent('Backspace',false);
     await expect.poll(()=>report.taskTrace.some(e=>e.line.includes('task_id='+start.taskId+' ')&&e.line.includes('turn=2')&&e.line.includes('action=Update applied=true')),{timeout:8000}).toBe(true);
     expect(await read()).toBe('abc正在识正在识别');
     expect(report.taskTrace.some(e=>e.line.includes('task_id='+start.taskId+' ')&&e.line.includes('keyboardHandedOff'))).toBe(true);
    },()=>keyEvent('AltRight',false),false,async()=>{
     await keyEvent('AltRight',true);await waitPhase(page,'recording');return invoke(page,'dictation_snapshot');
    });
    expect(await read()).toBe('abc正在识原生受控识别完成。');
    report.inputCompatibility.push({app:'Chromium',control,case:'held-backspace',eventSource:'tagged-injected-keyboard',exactDeletionPreserved:true,delivery:result.dictation.delivery});
    if(result.dictation.presentationId)await dismiss(page,{taskId:result.dictation.taskId,presentationId:result.dictation.presentationId});
   }
  } finally {
   await keyEvent('AltRight',false).catch(()=>{});
   await invoke(page,'shortcut_save',{config:{...(await invoke(page,'shortcut_status')).saved,binding:shortcut.binding,mode:shortcut.mode}});
  }
 }finally{await focusHelper.close();await targetBrowser.close();}
 await invoke(page,'shortcut_save',{config:{...(await invoke(page,'shortcut_status')).saved,binding:'AltRight',mode:'toggle'}});

for(const r of (await invoke(page,'history_list',{query:{search:'',page:1}})).records)await invoke(page,'history_remove',{id:r.id,revision:r.revision});expect((await invoke(page,'history_list',{query:{search:'',page:1}})).total).toBe(0);
 expect(report.connections.every(c=>c.started)).toBe(true);expect(report.protocolError).toBeUndefined();report.checks.push('auth/empty/service errors, content updates never invalidate native shrink; service failure retains recoverable recording; revision-guarded delete cleans registered files');
 await stop(page);
 await command(['test','--manifest-path','src-tauri/Cargo.toml','history_desktop_fixture','--','--ignored']);page=await start(5);
 const samples=[];for(let n=0;n<15;n++){const began=performance.now();const feed=await invoke(page,'history_browse',{query:{search:'中文',cursor:null}});samples.push(performance.now()-began);expect(feed.records).toHaveLength(50);expect(feed.total).toBe(10000);}
 samples.sort((a,b)=>a-b);report.historyIpc={size:10000,searchP95Ms:Math.round(samples[14]*100)/100};
 await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByText('10000 条',{exact:true})).toBeVisible();const scroll=page.getByLabel('历史记录列表',{exact:true});
 const renderStarted=performance.now();for(let n=0;n<30&&await page.getByRole('listitem').count()<1000;n++){await scroll.evaluate(node=>{node.scrollTop=node.scrollHeight;});await delay(150);}expect(await page.getByRole('listitem').count()).toBeGreaterThanOrEqual(1000);expect(new Set(await page.getByRole('listitem').evaluateAll(rows=>rows.map(row=>row.dataset.recordId))).size).toBe(await page.getByRole('listitem').count());report.historyBrowse1000Ms=Math.round(performance.now()-renderStarted);
 await scroll.evaluate(node=>{node.scrollTop=1200;});await delay(50);const readingTop=await scroll.evaluate(node=>node.scrollTop);await page.getByRole('link',{name:'首页',exact:true}).click();await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect.poll(()=>scroll.evaluate(node=>node.scrollTop)).toBe(readingTop);
 await invoke(page,'acceptance_main_size',{minimum:false});await expect.poll(()=>page.evaluate(()=>({width:innerWidth,height:innerHeight}))).toEqual({width:1120,height:760});
 await scroll.evaluate(node=>{node.scrollTop=0;});await page.getByRole('listitem').first().hover();await page.getByRole('listitem').first().getByRole('button',{name:'展开',exact:true}).click();await expect(page.getByRole('region',{name:'历史识别文字'})).toBeVisible();await page.screenshot({path:path.join(directory,'history-default.png')});
 report.historyScale=await invoke(page,'acceptance_main_size',{minimum:true});await expect.poll(()=>page.evaluate(()=>({width:innerWidth,height:innerHeight}))).toEqual({width:960,height:640});await expect(page.getByRole('region',{name:'历史识别文字'})).toBeVisible();
 const search=page.getByRole('textbox',{name:'搜索识别文字'});await search.focus();report.searchGeometry=await search.evaluate(node=>{const css=getComputedStyle(node),box=node.getBoundingClientRect();return {textHeight:node.clientHeight-parseFloat(css.paddingTop)-parseFloat(css.paddingBottom),lineHeight:parseFloat(css.lineHeight),outlineExtent:parseFloat(css.outlineWidth)+parseFloat(css.outlineOffset),left:box.left,right:box.right,containerRight:node.parentElement.getBoundingClientRect().right};});expect(report.searchGeometry.textHeight).toBeGreaterThanOrEqual(report.searchGeometry.lineHeight);expect(report.searchGeometry.outlineExtent).toBeLessThanOrEqual(0);expect(report.searchGeometry.right).toBeLessThanOrEqual(report.searchGeometry.containerRight);await page.screenshot({path:path.join(directory,'history-minimum-search.png')});
 await page.getByRole('listitem').first().getByRole('button',{name:'更多操作'}).click();await expect(page.getByRole('menu')).toBeVisible();await page.screenshot({path:path.join(directory,'history-minimum-menu.png')});await page.keyboard.press('Escape');await expect(page.getByRole('listitem').first().getByRole('button',{name:'更多操作'})).toBeFocused();
 await page.getByRole('listitem').first().getByRole('button',{name:'收起记录'}).click();await page.getByRole('button',{name:'保存设置',exact:true}).click();await expect(page.getByRole('combobox',{name:'录音保留时间'})).toBeVisible();await page.screenshot({path:path.join(directory,'history-minimum-settings.png')});expect(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth)).toBe(false);await page.getByRole('button',{name:'保存设置',exact:true}).first().click();
 await invoke(page,'acceptance_main_size',{minimum:false});
 report.checks.push('10k native summaries through IPC;1000 rows continuously loaded; cached reading restored by real routes');
 await stop(page);expect(readFileSync(path.join(data,'dictionary.sqlite')).includes(Buffer.from(key))).toBe(false);for(let round=1;round<=5;round++)for(const suffix of ['.log','-errors.log'])expect(readFileSync(path.join(directory,'round-'+round+suffix),'utf8')).not.toContain(key);if(report.compatibilityFailures?.length)throw Error('Actual input compatibility failures remain; see compatibilityFailures.');report.passed=true;
}catch(e){report.error=String(e).replaceAll(key,'[redacted]');report.errorStack=e.stack?.replaceAll(key,'[redacted]');if(browser){const main=browser.contexts().flatMap(c=>c.pages()).find(p=>!p.url().includes('surface=overlay'));if(main){await main.screenshot({path:path.join(directory,'failure.png')}).catch(()=>{});report.failureDialogs=await main.getByRole('dialog').count().catch(()=>null);const speech=await invoke(main,'speech_snapshot').catch(()=>null);report.failureSpeechPhase=speech?.phase??null;report.failureSpeechError=speech?.error??null;}}writeFileSync(path.join(directory,'pending-report.json'),JSON.stringify(report,null,2));process.exitCode=1;}finally{
 if(browser){const page=browser.contexts().flatMap(c=>c.pages()).find(p=>!p.url().includes('surface=overlay'));await stop(page).catch(()=>stop(null,false));}else await stop(null,false);for(const socket of sockets)socket.destroy();if(server)await new Promise(r=>server.close(r));
 try{await command(['test','--manifest-path','src-tauri/Cargo.toml','--lib','model_desktop_cleanup','--','--ignored']);report.credentialCleanup=true;}catch{report.credentialCleanup=false;report.passed=false;process.exitCode=1;}
 for(const name of ['fixture-ca.der','fixture-ca.pem','fixture-key.pem'])if(existsSync(path.join(data,name)))unlinkSync(path.join(data,name));writeFileSync(path.join(directory,'report.json'),JSON.stringify(report,null,2));writeFileSync(path.join(root,'.local','speech-desktop-report.json'),JSON.stringify(report,null,2));
 report.storage=finishValidation(root,storageContext,report,{processesClosed:!app&&(report.ownedEditors??[]).every(editor=>editor.exit!==null),credentialsCleared:report.credentialCleanup===true});
}
}
await run();
console.log(JSON.stringify(report,null,2));
