// Real native-worker probe. Audio is paced at capture speed; no host input is injected.
import {spawn} from 'node:child_process';
import {createHash,randomUUID} from 'node:crypto';
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import {once} from 'node:events';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';

const [workerArg,modelArg,audioArg,reportArg,device='cpu',expectedBackend]=process.argv.slice(2);
if(!reportArg||!['auto','cpu'].includes(device))throw Error('Usage: node scripts/test-local-asr.mjs worker.exe model.gguf mono16k.wav report.json [cpu|auto]');
const worker=path.resolve(workerArg),model=path.resolve(modelArg),audio=path.resolve(audioArg),report=path.resolve(reportArg);
const wav=readFileSync(audio);
if(wav.toString('ascii',0,4)!=='RIFF'||wav.toString('ascii',8,12)!=='WAVE')throw Error('Expected RIFF WAV');
let pcm,format;
for(let at=12;at+8<=wav.length;){
 const size=wav.readUInt32LE(at+4),end=at+8+size;
 if(end>wav.length)throw Error('Truncated WAV chunk');
 const kind=wav.toString('ascii',at,at+4);
 if(kind==='fmt ')format={encoding:wav.readUInt16LE(at+8),channels:wav.readUInt16LE(at+10),rate:wav.readUInt32LE(at+12),bits:wav.readUInt16LE(at+22)};
 if(kind==='data')pcm=wav.subarray(at+8,end);
 at=end+(size%2);
}
if(!pcm||format?.encoding!==1||format.channels!==1||format.rate!==16000||format.bits!==16)throw Error('Expected 16kHz mono PCM16');
const instance=randomUUID(),task=randomUUID();
const ids=[1,2].map(turn=>({task,turn,request:randomUUID()}));
const trace=[],stderr=[];
const child=spawn(worker,['--stdio'],{windowsHide:true,stdio:['pipe','pipe','pipe']});
const exited=once(child,'exit');
const boot=performance.now();
let pending=Buffer.alloc(0),terminalError;
const listeners=new Set();
const processEvent=value=>{
 trace.push({elapsedMs:performance.now()-boot,...value});
 if(value.kind==='error')terminalError=Error(value.code);
 for(const listener of [...listeners])listener();
};
child.stderr.on('data',chunk=>{if(stderr.reduce((n,v)=>n+v.length,0)<1024*1024)stderr.push(Buffer.from(chunk));});
child.stdout.on('data',chunk=>{
 pending=Buffer.concat([pending,chunk]);
 while(pending.length>=4){
  const size=pending.readUInt32LE(0);
  if(size<1||size>1024*1024+1){terminalError=Error('Oversized worker frame');child.kill();break;}
  if(pending.length<size+4)break;
  const frame=pending.subarray(4,size+4);pending=pending.subarray(size+4);
  try{if(frame[0]!==0)throw Error('Unexpected worker audio');processEvent(JSON.parse(frame.subarray(1).toString('utf8')));}
  catch(error){terminalError=error;child.kill();break;}
 }
});
child.on('error',error=>{terminalError=error;for(const listener of [...listeners])listener();});
child.on('exit',()=>{for(const listener of [...listeners])listener();});
async function waitFor(predicate,from=0,timeoutMs=15000){
 const find=()=>trace.slice(from).find(predicate);
 if(terminalError)throw terminalError;
 if(find())return find();
 return new Promise((resolve,reject)=>{
  const finish=(error,value)=>{clearTimeout(timer);listeners.delete(check);if(error)reject(error);else resolve(value);};
  const check=()=>{
   if(terminalError)finish(terminalError);
   else if(find())finish(null,find());
   else if(child.exitCode!==null)finish(Error('Worker exited before expected event'));
  };
  const timer=setTimeout(()=>finish(Error('Worker event deadline exceeded')),timeoutMs);
  listeners.add(check);check();
 });
}
async function writeFrame(tag,bytes){
 const head=Buffer.alloc(5);head.writeUInt32LE(bytes.length+1);head[4]=tag;
 if(!child.stdin.write(Buffer.concat([head,bytes])))await once(child.stdin,'drain');
}
const command=value=>writeFrame(0,Buffer.from(JSON.stringify(value)));
const runs=[];
let passed=false;
try{
 await command({kind:'hello',version:1,instance});
 const hello=await waitFor(value=>value.kind==='hello');
 if(hello.instance!==instance||hello.engine!=='0.1.3')throw Error('Worker identity/version mismatch');
 await command({kind:'prepare',id:ids[0],path:model,sha256:'41c99fa5fb6f3d35f68e79adc3e755eca2232a8d921178bd647b71194792b8fd',device});
 const prepared=await waitFor(value=>value.kind==='prepared'&&value.id.request===ids[0].request,0,60000);
 if(!prepared.streaming||(device==='cpu'&&prepared.backend.toLowerCase()!=='cpu'))throw Error('Actual worker backend differs');
 if(expectedBackend&&prepared.backend.toLowerCase()!==expectedBackend.toLowerCase())throw Error('Requested acceptance backend was not actually used');
 // Two streams share one worker/model. Finalize must settle the first before the next Ready.
 for(let turn=1;turn<=2;turn++){
  const id=ids[turn-1],start=trace.length;
  await command({kind:'start',id,language:'auto',rightContext:6});
  await waitFor(value=>value.kind==='ready'&&value.id.request===id.request,start);
  const began=performance.now();let sequence=0;
  for(let at=0;at<pcm.length;at+=3200){
   const chunk=pcm.subarray(at,at+3200),due=(at+chunk.length)/32;
   if(due>performance.now()-began)await delay(due-(performance.now()-began));
   const header=Buffer.from(JSON.stringify({id,sequence:++sequence})),size=Buffer.alloc(2);size.writeUInt16LE(header.length);
   await writeFrame(1,Buffer.concat([size,header,chunk]));
   if(terminalError)throw terminalError;
  }
  const finishing=performance.now();
  await command({kind:'finish',id});
  await waitFor(value=>value.kind==='finished'&&value.id.request===id.request,start,5000);
  const updates=trace.slice(start).filter(value=>value.kind==='update'&&value.id.request===id.request);
  const final=updates.at(-1),first=updates.find(value=>value.full);
  if(!final?.finalResult||updates.length<2||!first||first.elapsedMs>=finishing-boot)throw Error('No genuine pre-finish streaming updates');
  if(final.full!==final.confirmed)throw Error('Final authority/confirmation mismatch');
  runs.push({turn,durationMs:pcm.length/32,elapsedMs:performance.now()-began,finalizeMs:performance.now()-finishing,updates:updates.length,firstTextMs:first.elapsedMs-(began-boot),full:final.full});
 }
 await command({kind:'shutdown'});
 const [code]=await Promise.race([exited,delay(3000).then(()=>{throw Error('Worker did not release resources');})]);
 if(code!==0)throw Error(`Worker exit ${code}`);
 passed=true;
}catch(error){terminalError=error;}
finally{
 if(child.exitCode===null){child.kill();await Promise.race([exited,delay(2000)]).catch(()=>{});}
 mkdirSync(path.dirname(report),{recursive:true});
 writeFileSync(report,JSON.stringify({kind:'real-model-native-pipe-probe',passed,worker,workerSha256:createHash('sha256').update(readFileSync(worker)).digest('hex'),model,audio,syntheticAudio:audio.includes('sapi'),device,runs,error:terminalError?.message??null,resourcesClosed:child.exitCode!==null||child.signalCode!==null,trace},null,2)+'\n');
 writeFileSync(`${report}.stderr.log`,Buffer.concat(stderr));
}
if(!passed)throw terminalError??Error('Local streaming probe failed');
console.log(JSON.stringify({passed,runs,report}));
