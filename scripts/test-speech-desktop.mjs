// Actual WebView, Windows vault, microphone and disk; ASR text comes from an isolated TLS fixture.
// No production trust override and no supplier request. Reports stay under .local.
import {chromium,expect} from '@playwright/test';
import {build as buildFixture} from 'vite';
import {spawn} from 'node:child_process';
import {randomUUID,createHash} from 'node:crypto';
import {createServer} from 'node:https';
import {gzipSync,gunzipSync} from 'node:zlib';
import {createWriteStream,mkdirSync,readFileSync,writeFileSync,unlinkSync,existsSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..'),id=randomUUID();
const directory=path.join(root,'.local','data-acceptance',id),data=path.join(directory,'data');mkdirSync(data,{recursive:true});
const exe=path.join(root,'src-tauri','target','release','desktop-acceptance.exe'),port=9241,key='fixture-'+randomUUID();
const report={id,passed:false,checks:[],connections:[],handshakes:[]},delay=ms=>new Promise(r=>setTimeout(r,ms));
let app,browser,server,mode='normal',expectedProtocol='qwen',expectedModel='qwen-audio-3.1-asr-flash-streaming';const sockets=new Set();
async function command(args){const child=spawn('cargo',args,{cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:id},stdio:['ignore','pipe','pipe']});const log=createWriteStream(path.join(directory,'fixture.log'),{flags:'a'});child.stdout.pipe(log);child.stderr.pipe(log);if(await new Promise((r,j)=>{child.on('exit',r);child.on('error',j);})!==0)throw Error('Fixture operation failed');}
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
 const result=(text,end)=>send('result-generated',{output:{sentence:{sentence_id:1,sentence_end:end,text,begin_time:0,end_time:end?connection.bytes/32:null}}});
 const binaryPayload=(value,last=false)=>{const bytes=gzipSync(Buffer.from(JSON.stringify(value)));connection.responseSequence=(connection.responseSequence??0)+1;const header=Buffer.alloc(12);header.set([0x11,last?0x93:0x91,0x11,0]);header.writeInt32BE(last?-1:connection.responseSequence,4);header.writeUInt32BE(bytes.length,8);socket.write(frame(Buffer.concat([header,bytes]),2));};
 const binaryResult=(text,end,last=false)=>binaryPayload({code:20000000,result:{text,utterances:text?[{text,definite:end,start_time:0,end_time:Math.round(connection.bytes/32)}]:[]}},last);
 function consume(opcode,payload){
  if(doubao&&opcode===2){
   const kind=payload[1]>>4,flags=payload[1]&15;expect(payload.readUInt32BE(4)).toBe(payload.length-8);const body=gunzipSync(payload.subarray(8));
   if(kind===1){const value=JSON.parse(body.toString());expect(value.audio).toMatchObject({format:'pcm',rate:16000,bits:16,channel:1});expect(value.request).toMatchObject({model_name:'bigmodel',result_type:'full',show_utterances:true});connection.model='Doubao-Seed-ASR-2.0';connection.started=true;binaryResult('',false);return;}
   expect(kind).toBe(2);expect(connection.started).toBe(true);
   if(flags&2){connection.finished=true;if(connection.mode==='pending')return;if(connection.mode==='empty'||connection.mode==='empty-ack'){binaryResult('',true,true);return;}if(connection.mode==='malformed-terminal'){binaryPayload({code:20000000,result:{text:123}},true);return;}const complete=()=>{const text=connection.mode==='single-a'?'a':connection.mode==='unicode'?'中文，🙂\n第二行 e\u0301。':'原生受控识别完成。';binaryResult(text,true);binaryResult('旧暂定不能覆盖。',false);binaryResult(text,true,true);};if(connection.mode==='delayed-final')setTimeout(complete,650);else complete();return;}
   connection.frames++;connection.bytes+=body.length;if(connection.frames===1)binaryResult('正在识别',false);return;
  }

  if(opcode===8){socket.end(frame(payload,8));return;}if(opcode===9){socket.write(frame(payload,10));return;}
  if(opcode===2){if(!connection.started)throw Error('Audio arrived before task-started');connection.frames++;connection.bytes+=payload.length;if(connection.frames===1){if(connection.mode==='failure'){socket.write(frame({header:{task_id:task,event:'task-failed',error_code:'SERVICE_ERROR',error_message:'PRIVATE FIXTURE ERROR'}}));return;}result('正在识别',false);}return;}
  if(opcode!==1)return;const value=JSON.parse(payload.toString());if(value.header.action==='run-task'){task=value.header.task_id;expect(value.payload.parameters.sample_rate).toBe(16000);expect(value.payload.parameters.format).toBe('pcm');expect(value.payload.model).toBe(expectedModel);connection.model=value.payload.model;connection.started=true;send('task-started');}
  else if(value.header.action==='finish-task'){connection.finished=true;if(connection.mode==='pending')return;if(connection.mode==='empty'){send('task-finished');return;}result('原生受控识别完成。',true);result('重复最终结果不应覆盖。',true);send('task-finished');}
 }
 function receive(chunk){buffer=Buffer.concat([buffer,chunk]);try{while(buffer.length>=2){const opcode=buffer[0]&15,fin=(buffer[0]&128)!==0,masked=(buffer[1]&128)!==0;let length=buffer[1]&127,offset=2;if(length===126){if(buffer.length<4)return;length=buffer.readUInt16BE(2);offset=4;}else if(length===127){if(buffer.length<10)return;length=Number(buffer.readBigUInt64BE(2));offset=10;}if(length>1024*1024)throw Error('oversize frame');if(masked)offset+=4;if(buffer.length<offset+length)return;const payload=Buffer.from(buffer.subarray(offset,offset+length));if(masked){const mask=buffer.subarray(offset-4,offset);for(let n=0;n<length;n++)payload[n]^=mask[n%4];}buffer=buffer.subarray(offset+length);if(opcode===0){fragment=Buffer.concat([fragment,payload]);if(fin){consume(fragmentOpcode,fragment);fragment=Buffer.alloc(0);}}else if(!fin&&opcode<8){fragmentOpcode=opcode;fragment=payload;}else consume(opcode,payload);}}catch(e){report.protocolError=String(e);socket.destroy();}}
 socket.on('data',receive);if(head.length)receive(Buffer.alloc(0));
}
async function processMetrics(){
 const taskPid=app.pid;if(!Number.isSafeInteger(taskPid)||taskPid<=0)throw Error('Invalid owned process');
 const code=`$taskProcess=Get-Process -Id ${taskPid}; if ($taskProcess.Path -ne '${exe.replaceAll("'","''")}') {throw 'Process ownership changed'}; [pscustomobject]@{cpuSeconds=$taskProcess.TotalProcessorTime.TotalSeconds;workingSetBytes=$taskProcess.WorkingSet64;privateBytes=$taskProcess.PrivateMemorySize64} | ConvertTo-Json -Compress`;
 const probe=spawn('powershell',['-NoProfile','-Command',code],{windowsHide:true,stdio:['ignore','pipe','pipe']});let output='';probe.stdout.on('data',chunk=>output+=chunk);if(await new Promise((r,j)=>{probe.on('exit',r);probe.on('error',j);})!==0)throw Error('Owned process metrics unavailable');return JSON.parse(output);
}
const invoke=async(page,command,args={})=>{const result=await page.evaluate(async({command,args})=>{try{return {value:await window.__TAURI_INTERNALS__.invoke(command,args)};}catch(e){return {error:e.code??'unavailable'};}},{command,args});if(result.error)throw Error(command+':'+result.error);return result.value;};
async function readModels(page){for(let n=0;n<30;n++){try{const status=await invoke(page,'model_status',{retry:false});if(status.state==='preparing'){await delay(100);continue;}expect(status.state).toBe('ready');return await invoke(page,'model_overview');}catch(e){if(n===29||!/:busy$|:preparing$/.test(String(e)))throw e;await delay(100);}}throw Error('Model initialization did not become ready');}
const rejected=(page,command,args={})=>page.evaluate(async({command,args})=>{try{await window.__TAURI_INTERNALS__.invoke(command,args);return null;}catch(e){return e;}},{command,args});
async function start(round){
 if(await fetch('http://127.0.0.1:'+port+'/json/version').then(()=>true).catch(()=>false))throw Error('Test port occupied');
 app=spawn(exe,[],{cwd:root,windowsHide:true,env:{...process.env,SAYWEAVE_DATA_ACCEPTANCE_ID:id,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port='+port+' --remote-debugging-address=127.0.0.1'},stdio:['ignore','pipe','pipe']});app.stdout.pipe(createWriteStream(path.join(directory,`round-${round}.log`)));app.stderr.pipe(createWriteStream(path.join(directory,`round-${round}-errors.log`)));
 report.processBindings??=[];report.processBindings.push({round,pid:app.pid,exe,sha256:createHash('sha256').update(readFileSync(exe)).digest('hex'),data,logSource:'launched-process-stderr'});
 let diagnostic='';app.stderr.on('data',chunk=>{diagnostic+=chunk.toString('utf8');let at;while((at=diagnostic.indexOf('\n'))>=0){const line=diagnostic.slice(0,at);diagnostic=diagnostic.slice(at+1);if(/operation=(input_target_event|input_target_identity|input_capture|input_delivery|input_binding|voice_window|dictation_phase|composition_edit|dictation_turn)\b/.test(line)){report.taskTrace??=[];report.taskTrace.push({round,line});}}});
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
 let buffer='',queue=[],waiting=[];child.stdout.on('data',data=>{buffer+=data.toString('utf8');let at;while((at=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,at);buffer=buffer.slice(at+1);const value=JSON.parse(line);if(waiting.length)waiting.shift()(value);else queue.push(value);}});
 const next=()=>queue.length?Promise.resolve(queue.shift()):Promise.race([new Promise(resolve=>waiting.push(resolve)),delay(3000).then(()=>{throw Error('Owned editor response timeout');})]);
 await next();return {command:async value=>{child.stdin.write(JSON.stringify(value)+'\n');const result=await next();if(['focus','focus-process'].includes(value.action))await delay(80);return result;},close:async()=>{if(child.exitCode===null){child.stdin.write('{"action":"exit"}\n');for(let n=0;n<20&&child.exitCode===null;n++)await delay(50);if(child.exitCode===null)child.kill();}}};
}
async function nativeOverlayClick(view,editor,name){
 const owned=await editor.command({action:'inspect-overlay',process:app.pid});expect(owned.overlayWindow).toBeGreaterThan(0);
 const box=await view.getByRole('button',{name,exact:true}).boundingBox();expect(box).not.toBeNull();
 const scale=await view.evaluate(()=>devicePixelRatio);
 const click=await editor.command({action:'click-window',process:app.pid,window:owned.overlayWindow,x:Math.round((box.x+box.width/2)*scale),y:Math.round((box.y+box.height/2)*scale)});
 expect(click.clickCheck.pointRootMatches).toBe(true);expect(click.clicked).toBe(2);return click;
}
async function externalRound(page,finish,end,interrupted=false){
 const start=await invoke(page,'dictation_start');await waitPhase(page,'recording');
 for(const command of ['dictation_control','speech_control'])expect((await rejected(page,command,{taskId:'stale-task',action:'cancel'})).code).toBe('staleSession');
 await finish(start);
 if(!['completed','failed','cancelled'].includes((await invoke(page,'speech_snapshot')).phase)){
  if(end)await end(start);else await invoke(page,'dictation_control',{taskId:start.taskId,action:'finish'});
 }
 if(mode==='delayed-final'){await expect.poll(async()=>(await invoke(page,'dictation_snapshot')).presentation,{timeout:3000}).toBe('recognizing');const view=await overlayPage();await expect(view.getByText('识别中',{exact:true})).toBeVisible();await expect(view.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);}
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
try{
 await command(['test','--manifest-path','src-tauri/Cargo.toml','model_desktop_fixture','--','--ignored']);server=createServer({cert:readFileSync(path.join(data,'fixture-ca.pem')),key:readFileSync(path.join(data,'fixture-key.pem'))});server.on('upgrade',websocket);await new Promise(r=>server.listen(0,'127.0.0.1',r));const base='https://localhost:'+server.address().port;
 let page=await start(1);await expect(page.getByRole('link',{name:'组件展示'})).toHaveCount(0);await page.getByRole('button',{name:'配置模型',exact:true}).click();const dialog=page.getByRole('dialog');await delay(250);await page.screenshot({path:path.join(directory,'settings-speech-native.png')});await dialog.getByRole('button',{name:'更多设置'}).click();await dialog.getByLabel('名称',{exact:true}).fill('隔离语音验收');await dialog.getByLabel('服务地址（URL）',{exact:true}).fill(base);await dialog.getByLabel('API 密钥',{exact:true}).fill(key);await dialog.getByRole('button',{name:'保存',exact:true}).click();await expect(dialog.getByText('已保存。')).toBeVisible();await dialog.getByRole('button',{name:'关闭',exact:true}).click();await expect(dialog).toHaveCount(0);
 const devices=await invoke(page,'speech_devices');report.microphoneCount=devices.length;expect(devices.some(d=>d.isDefault)).toBe(true);
 const began=performance.now();await page.getByRole('button',{name:'开始录音',exact:true}).click();const live=await waitPhase(page,'recording');await expect(page.getByRole('region',{name:'识别文字',exact:true})).toContainText('正在识别');report.firstFixtureTextMs=Math.round(performance.now()-began);
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
 await page.getByRole('link',{name:'历史记录',exact:true}).click();await expect(page.getByText('1 条',{exact:true})).toBeVisible();const row=page.getByRole('listitem').first();await row.hover();await row.getByRole('button',{name:'播放录音',exact:true}).click();await expect(row.getByRole('region',{name:'历史识别文字'})).toContainText('原生受控识别完成。');await expect(page.getByRole('dialog')).toHaveCount(0);await row.getByRole('button',{name:'停止播放'}).click();await page.screenshot({path:path.join(directory,'history.png')});

 await page.getByRole('button',{name:'保存设置',exact:true}).click();await page.getByRole('combobox',{name:'录音保留时间'}).selectOption('7');await page.getByRole('link',{name:'首页',exact:true}).click();await expect(page.getByRole('alertdialog',{name:'有未保存的修改'})).toBeVisible();await page.getByRole('button',{name:'继续填写'}).click();await page.getByRole('button',{name:'保存设置',exact:true}).last().click();await expect(page.getByText('已保存，下一次录音生效。')).toBeVisible();await page.getByRole('button',{name:'保存设置',exact:true}).first().click();
 report.checks.push('main-only bounded history summaries, inline native playback and retention draft protection');report.viewport=await page.evaluate(()=>({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,overflow:document.documentElement.scrollWidth>innerWidth}));expect(report.viewport.overflow).toBe(false);await stop(page);page=await start(2);
 overlay=await overlayPage();history=await invoke(page,'history_list',{query:{search:'',page:1}});expect(history.records[0].audio.status).toBe('ready');expect(history.records[0].attempts[0].text).toBe('原生受控识别完成。');const view=await readModels(page),config=view.configs[0];
 const retryArgs={id:record.id,configId:config.id,configRevision:config.revision};mode='pending';const retry=await invoke(page,'history_retry',retryArgs);await waitPhase(page,'processing');await delay(100);await invoke(page,'speech_control',{taskId:retry.taskId,action:'cancel'});await waitPhase(page,'cancelled');expect((await invoke(page,'history_detail',{id:record.id})).currentAttempt).toBe(record.currentAttempt);
 mode='failure';await invoke(page,'history_retry',retryArgs);await waitPhase(page,'failed');const failedRetry=await invoke(page,'speech_snapshot');expect(failedRetry.canRetry).toBe(true);mode='normal';const recovered=await invoke(page,'speech_retry_current',{taskId:failedRetry.taskId});expect(recovered.recordId).toBe(record.id);await waitPhase(page,'completed');const updated=await invoke(page,'history_detail',{id:record.id});expect(updated.attempts).toHaveLength(4);const retryFeed=await invoke(page,'history_browse',{query:{search:'',cursor:null}});expect(retryFeed.records.find(r=>r.id===record.id).displayAttempt).toBe(updated.currentAttempt);expect(updated.currentAttempt).not.toBe(record.currentAttempt);expect((await invoke(page,'history_usage')).count).toBe(1);
 mode='pending';const cancelled=await invoke(page,'speech_start');await waitPhase(page,'recording');await delay(200);await invoke(page,'speech_control',{taskId:cancelled.taskId,action:'finish'});await waitPhase(page,'processing');const layout=await invoke(page,'runtime_snapshot');await delay(700);expect((await invoke(page,'runtime_snapshot')).revision).toBe(layout.revision);expect(await overlay.evaluate(()=>({width:innerWidth,height:innerHeight}))).toEqual({width:108,height:64});await invoke(page,'speech_control',{taskId:cancelled.taskId,action:'cancel'});await waitPhase(page,'cancelled');expect((await invoke(page,'history_list',{query:{search:'',page:1}})).total).toBe(1);
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
 const beforeMalformed=await invoke(page,'history_detail',{id:doubaoRecord.id});mode='malformed-terminal';await invoke(page,'history_retry',{id:doubaoRecord.id,configId:doubaoConfig.id,configRevision:doubaoConfig.revision});await waitPhase(page,'failed');expect((await invoke(page,'speech_snapshot')).error).toBe('protocol');expect((await invoke(page,'history_detail',{id:doubaoRecord.id})).currentAttempt).toBe(beforeMalformed.currentAttempt);await expect(page.getByRole('group',{name:'识别失败',exact:true})).toBeVisible();await expect(overlay.getByRole('group',{name:'识别失败',exact:true})).toHaveCount(1);await expect(page.getByText('演示失败',{exact:true})).toHaveCount(0);
 report.checks.push('negative terminal sentinel accepts final text and empty acknowledgement; malformed terminal stays failed, preserves prior success and uses real recognition labels in both windows');
 mode='resource-denied';const handshakes=report.handshakes.length;await invoke(page,'history_retry',{id:doubaoRecord.id,configId:doubaoConfig.id,configRevision:doubaoConfig.revision});await waitPhase(page,'completed');expect(report.handshakes.slice(handshakes).map(h=>h.resource)).toEqual(['volc.seedasr.sauc.duration','volc.seedasr.sauc.concurrent']);
 mode='auth';const authHandshakes=report.handshakes.length;await invoke(page,'speech_start');await waitPhase(page,'failed');expect((await invoke(page,'speech_snapshot')).error).toBe('auth');expect(report.handshakes.length-authHandshakes).toBe(1);
 mode='pending';const doubaoCancel=await invoke(page,'history_retry',{id:doubaoRecord.id,configId:doubaoConfig.id,configRevision:doubaoConfig.revision});await waitPhase(page,'processing');await invoke(page,'speech_control',{taskId:doubaoCancel.taskId,action:'cancel'});await waitPhase(page,'cancelled');
 report.checks.push('Doubao model UI mapping, cross-service key reset, native binary/gzip stream, partial/final deduplication, WAV persistence and retry; explicit resource-only fallback; auth does not retry; cancellation preserves history');
 await stop(page);page=await start(4);const doubaoReopened=(await readModels(page)).configs[0];expect(doubaoReopened.protocol).toBe('doubao-asr-streaming');expect(doubaoReopened.credentialStatus).toBe('present');expect((await invoke(page,'history_detail',{id:doubaoRecord.id})).audio.status).toBe('ready');
 report.checks.push('Doubao selection, credential and history survive a real process restart');
 mode='normal';
 const keys=await invoke(page,'shortcut_status');expect(keys.active.binding).toBe('AltRight');
 const captureToken=randomUUID();const capturingKeys=await invoke(page,'shortcut_capture',{begin:true,token:captureToken});expect(capturingKeys.captureToken).toBe(captureToken);expect(capturingKeys.capturing).toBe(true);expect(capturingKeys.sequence).toBeGreaterThan(keys.sequence);
 const stoppedCapture=await invoke(page,'shortcut_capture',{begin:false});expect(stoppedCapture.captureToken).toBe(captureToken);expect(stoppedCapture.capturing).toBe(false);expect(stoppedCapture.sequence).toBeGreaterThan(capturingKeys.sequence);
 expect((await rejected(page,'shortcut_capture',{begin:true,token:'invalid'})).code).toBe('invalidShortcut');
 const savedKeys=await invoke(page,'shortcut_save',{config:{...keys.saved,binding:'Control+Shift+F10',mode:'hold'}});expect(savedKeys.active.mode).toBe('hold');
 overlay=await overlayPage();
 for(const command of ['shortcut_status','shortcut_save','history_delivery'])expect((await rejected(overlay,command,command==='history_delivery'?{id:doubaoRecord.id}:command==='shortcut_save'?{config:savedKeys.saved}:{})).code).toBe('forbidden');
 const editor=await ownedEditor();try{
  expect((await editor.command({action:'focus',text:'before tail',start:7,end:11})).foreground).toBe(true);
  await invoke(page,'runtime_window',{surface:'overlay',action:'show'});mode='delayed-final';
  const result=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text,{timeout:5000}).toBe('before 正在识别');const before=await editor.command({action:'read'});expect(before.foreground).toBe(true);await invoke(page,'shortcut_capture',{begin:false});await invoke(page,'shortcut_capture',{begin:false});await delay(180);const after=await editor.command({action:'read'});report.externalFocus={beforeProcess:before.foregroundProcess,afterProcess:after.foregroundProcess,applicationProcess:app.pid};report.streamingInput={beforeFinishActualText:true,clipboardUsed:false,taskId:(await invoke(page,'dictation_snapshot')).taskId};expect(after.foreground).toBe(true);},async()=>{report.nativeEndClick=await nativeOverlayClick(overlay,editor,'结束录音');});
  report.externalEdit=result.dictation.delivery;expect(result.dictation.delivery.state).toBe('confirmed');expect((await rejected(page,'dictation_copy',{taskId:'stale-task',presentationId:randomUUID()})).code).toBe('staleSession');expect((await editor.command({action:'read'})).text).toBe('before 原生受控识别完成。');
  expect(result.dictation.presentation).toBe('completed');await expect(overlay.getByRole('button',{name:'复制',exact:true})).toHaveCount(0);await expect.poll(async()=>(await invoke(page,'runtime_snapshot')).windows.overlay,{timeout:2500}).toBe(false);expect((await invoke(page,'dictation_snapshot')).presentation).toBe('idle');mode='normal';
  expect((await rejected(page,'speech_retry_current',{taskId:'stale-displayed-task'})).code).toBe('staleSession');
  expect((await invoke(page,'history_delivery',{id:result.speech.recordId}))[0].state).toBe('confirmed');
  expect((await editor.command({action:'read'})).foreground).toBe(true);
  report.overlayInsertion=[];
  mode='unicode';
  for(const sample of [{before:'前面尾部',start:2,end:4},{before:'',start:0,end:0},{before:'a😀b',start:1,end:3}]){
   await editor.command({action:'focus',text:sample.before,start:sample.start,end:sample.end});
   const result=await externalRound(page,async()=>delay(180),async()=>nativeOverlayClick(overlay,editor,'结束录音'));
   const actual=await editor.command({action:'read'});
   expect(actual.text.replace(/\r\n/g,'\n')).toBe(sample.before.slice(0,sample.start)+'中文，🙂\n第二行 e\u0301。'+sample.before.slice(sample.end));
   expect(actual.foreground).toBe(true);expect(result.dictation.delivery.state).toBe('confirmed');
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
  }finally{await legacy.close();}
  const other=await ownedEditor();
  try{
   await editor.command({action:'focus',text:'protected',start:9,end:9});
   const away=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('protected正在识别');expect((await other.command({action:'focus'})).foreground).toBe(true);expect((await editor.command({action:'focus'})).foreground).toBe(true);},undefined,true);
   expect(away.dictation.delivery.state).not.toBe('confirmed');
   expect((await editor.command({action:'read'})).text).toBe('protected正在识别');
   expect((await other.command({action:'read'})).text).toBe('');
   await invoke(page,'dictation_dismiss',{taskId:away.dictation.taskId,presentationId:away.dictation.presentationId});
   report.externalSwitchBack=away.dictation.delivery;
  }finally{await other.close();}
  await editor.command({action:'focus',text:'protected',start:9,end:9});
  const mainSwitch=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('protected正在识别');await invoke(page,'runtime_window',{surface:'main',action:'show'});await delay(100);expect((await editor.command({action:'read'})).foregroundProcess).toBe(app.pid);expect((await editor.command({action:'focus'})).foreground).toBe(true);},undefined,true);
  expect(mainSwitch.dictation.delivery.state).not.toBe('confirmed');
  expect((await editor.command({action:'read'})).text).toBe('protected正在识别');
  await invoke(page,'dictation_dismiss',{taskId:mainSwitch.dictation.taskId,presentationId:mainSwitch.dictation.presentationId});report.mainSwitchBack=mainSwitch.dictation.delivery;
  await editor.command({action:'focus',text:'protected',start:9,end:9});
  const caret=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('protected正在识别');await editor.command({action:'selection',start:0,end:0});},undefined,true);
  expect(caret.dictation.delivery.state).not.toBe('confirmed');
  expect((await editor.command({action:'read'})).text).toBe('protected正在识别');
  await invoke(page,'dictation_dismiss',{taskId:caret.dictation.taskId,presentationId:caret.dictation.presentationId});report.caretChange=caret.dictation.delivery;
  expect((await editor.command({action:'focus',text:'initial',start:7,end:7})).foreground).toBe(true);
  const changed=await externalRound(page,async()=>{await expect.poll(async()=>(await editor.command({action:'read'})).text).toBe('initial正在识别');await editor.command({action:'focus',text:'user changed',start:0,end:0});},undefined,true);
  expect(changed.dictation.delivery.state).not.toBe('confirmed');await invoke(page,'dictation_dismiss',{taskId:changed.dictation.taskId,presentationId:changed.dictation.presentationId});expect((await editor.command({action:'read'})).text).toBe('user changed');
  report.checks.push('ASR partial enters actual native selection before finish; real Win32 mouse end click updates one range with final Unicode/multiline/emoji and retains foreground; external/main switches preserve handed-off visible text without further writes after switching back; programmatic target edits retain user text; delayed provider tail waits and overlay hides; journal and overlay permissions verified');
 }finally{await editor.close();}
 const targetBrowser=await chromium.launch({headless:false,args:['--force-renderer-accessibility']});
 const focusHelper=await ownedEditor();
 try{const session=await targetBrowser.newBrowserCDPSession();const process=(await session.send('SystemInfo.getProcessInfo')).processInfo.find(p=>p.type==='browser').id;await session.detach();const target=await targetBrowser.newPage();await target.setContent('<input id="input"/><textarea id="area"></textarea><div id="editable" contenteditable="true"></div><textarea id="readonly" readonly></textarea><input id="password" type="password"/><div id="mixed" contenteditable="true"></div><div id="body" tabindex="0"></div>');
  report.inputCompatibility=[];
  for(const id of ['input','area','editable','readonly','password','mixed','body']){const nonce='fixture-'+randomUUID(),original=id==='mixed'?nonce+' fixed':nonce;await target.locator('#'+id).evaluate((node,nonce)=>{if('value'in node)node.value=nonce;else if(node.id==='mixed')node.innerHTML=nonce+'<span contenteditable="false"> fixed</span>';else node.textContent=nonce;},nonce);await target.bringToFront();await target.locator('#'+id).focus();await target.locator('#'+id).evaluate(node=>{if(node instanceof HTMLInputElement||node instanceof HTMLTextAreaElement)node.setSelectionRange(node.value.length,node.value.length);else{const selection=getSelection(),range=document.createRange();if(node.id==='mixed'){range.setStart(node.firstChild,node.firstChild.textContent.length);range.collapse(true);}else{range.selectNodeContents(node);range.collapse(false);}selection.removeAllRanges();selection.addRange(range);}});
   expect((await focusHelper.command({action:'focus-process',process})).foregroundProcess).toBe(process);await delay(200);const result=await externalRound(page,async()=>delay(150),['input','area','editable','mixed'].includes(id)?async()=>nativeOverlayClick(overlay,focusHelper,'结束录音'):undefined);const actual=await target.locator('#'+id).evaluate(node=>'value'in node?node.value:node.textContent);
   const expected=id==='mixed'?nonce+'原生受控识别完成。 fixed':original+'原生受控识别完成。';if(['readonly','password','body'].includes(id)){expect(result.dictation.delivery.submission).toBe('notSubmitted');expect(actual).toBe(original);}else{expect(actual,'Writable '+id+' must actually receive text: '+JSON.stringify(result.dictation.delivery)).toBe(expected);expect(result.dictation.delivery.submission).toBe('submitted');}
   if(result.dictation.delivery.state==='confirmed'||result.dictation.delivery.state==='unconfirmed')expect(result.dictation.presentation).not.toBe('result');
   if(result.dictation.presentation==='result'){await expect(overlay.getByRole('button',{name:'复制',exact:true})).toBeVisible();await expect(overlay.getByRole('button',{name:'关闭',exact:true})).toBeVisible();}
   if(result.dictation.presentationId)await invoke(page,'dictation_dismiss',{taskId:result.dictation.taskId,presentationId:result.dictation.presentationId});
   report.inputCompatibility.push({app:'Chromium',control:id,presentation:result.dictation.presentation,delivery:result.dictation.delivery.state,error:result.dictation.delivery.error,observedWrite:actual===expected});
  }
 // Repeated bounded context is not enough to prove either location or a successful paste.
  await target.bringToFront();await target.locator('#area').evaluate(node=>{node.value='a'.repeat(6000);node.focus();node.setSelectionRange(3000,3000);});
  await focusHelper.command({action:'focus-process',process});await delay(100);
  const moved=await externalRound(page,async()=>{await target.locator('#area').evaluate(node=>node.setSelectionRange(3500,3500));await delay(100);});
  expect(moved.dictation.delivery.submission).toBe('notSubmitted');expect(moved.dictation.delivery.error).toBe('targetChanged');expect(await target.locator('#area').inputValue()).toBe('a'.repeat(6000));
  await invoke(page,'dictation_dismiss',{taskId:moved.dictation.taskId,presentationId:moved.dictation.presentationId});
  mode='single-a';await target.locator('#area').evaluate(node=>{node.value='a'.repeat(3000);node.maxLength=3000;node.focus();node.setSelectionRange(3000,3000);});await focusHelper.command({action:'focus-process',process});await delay(100);
  const refused=await externalRound(page,async()=>delay(100));expect(await target.locator('#area').inputValue()).toBe('a'.repeat(3000));expect(refused.dictation.delivery.submission).toBe('submitted');expect(refused.dictation.delivery.state).toBe('unconfirmed');expect(refused.dictation.delivery.verification).not.toBe('confirmed');
  report.inputProtection={moved:moved.dictation.delivery,refused:refused.dictation.delivery};
  await invoke(page,'dictation_dismiss',{taskId:refused.dictation.taskId,presentationId:refused.dictation.presentationId});mode='normal';
 const bundle=await buildFixture({configFile:false,logLevel:'error',build:{write:false,lib:{entry:path.join(root,'tests/fixtures/input-target.ts'),name:'InputFixture',formats:['iife']}}});
  await target.addScriptTag({content:(Array.isArray(bundle)?bundle[0]:bundle).output[0].code});
  for(const sample of [{text:'前面尾部',start:2,end:4},{text:'很长的文档。'.repeat(8000),start:24000,end:24000}]){
   await target.bringToFront();await target.evaluate(sample=>window.fixtureEditor.set(sample.text,sample.start,sample.end),sample);
   expect((await focusHelper.command({action:'focus-process',process})).foregroundProcess).toBe(process);await delay(100);
   const result=await externalRound(page,async()=>delay(100),async()=>nativeOverlayClick(overlay,focusHelper,'结束录音'));const actual=await target.evaluate(()=>window.fixtureEditor.read());
   expect(actual,'ProseMirror must receive text: '+JSON.stringify(result.dictation.delivery)).toBe(sample.text.slice(0,sample.start)+'原生受控识别完成。'+sample.text.slice(sample.end));
   expect(result.dictation.delivery.submission).toBe('submitted');
   report.inputCompatibility.push({app:'Chromium',control:'ProseMirror',length:sample.text.length,delivery:result.dictation.delivery});
   if(result.dictation.presentationId)await invoke(page,'dictation_dismiss',{taskId:result.dictation.taskId,presentationId:result.dictation.presentationId});
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
   expect(actual.replace(/\r\n/g,'\n')).toBe(sample.before.slice(0,sample.start)+'中文，🙂\n第二行 e\u0301。'+sample.before.slice(sample.end));
   expect(result.dictation.delivery.state,'Unicode range confirmation: '+JSON.stringify(result.dictation.delivery)).toBe('confirmed');
   report.inputCompatibility.push({app:'Chromium',control,case:'unicode-multiline',start:sample.start,beforeLength:sample.before.length,delivery:result.dictation.delivery});
   if(result.dictation.presentationId)await invoke(page,'dictation_dismiss',{taskId:result.dictation.taskId,presentationId:result.dictation.presentationId});
  }
  mode='normal';
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
 await stop(page);expect(readFileSync(path.join(data,'dictionary.sqlite')).includes(Buffer.from(key))).toBe(false);for(let round=1;round<=5;round++)for(const suffix of ['.log','-errors.log'])expect(readFileSync(path.join(directory,'round-'+round+suffix),'utf8')).not.toContain(key);report.passed=true;
}catch(e){report.error=String(e).replaceAll(key,'[redacted]');report.errorStack=e.stack?.replaceAll(key,'[redacted]');if(browser){const main=browser.contexts().flatMap(c=>c.pages()).find(p=>!p.url().includes('surface=overlay'));if(main){await main.screenshot({path:path.join(directory,'failure.png')}).catch(()=>{});report.failureDialogs=await main.getByRole('dialog').count().catch(()=>null);const speech=await invoke(main,'speech_snapshot').catch(()=>null);report.failureSpeechPhase=speech?.phase??null;report.failureSpeechError=speech?.error??null;}}process.exitCode=1;}finally{
 if(browser){const page=browser.contexts().flatMap(c=>c.pages()).find(p=>!p.url().includes('surface=overlay'));await stop(page).catch(()=>stop(null,false));}else await stop(null,false);for(const socket of sockets)socket.destroy();if(server)await new Promise(r=>server.close(r));
 try{await command(['test','--manifest-path','src-tauri/Cargo.toml','model_desktop_cleanup','--','--ignored']);report.credentialCleanup=true;}catch{report.credentialCleanup=false;report.passed=false;process.exitCode=1;}
 for(const name of ['fixture-ca.der','fixture-ca.pem','fixture-key.pem'])if(existsSync(path.join(data,name)))unlinkSync(path.join(data,name));writeFileSync(path.join(directory,'report.json'),JSON.stringify(report,null,2));writeFileSync(path.join(root,'.local','speech-desktop-report.json'),JSON.stringify(report,null,2));
}
console.log(JSON.stringify(report,null,2));
