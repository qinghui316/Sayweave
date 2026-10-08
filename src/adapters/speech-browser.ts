import {SpeechError,initialSpeech,type SpeechPort,type SpeechSnapshot,type SpeechMeter,type RecordingPreferences} from '../contracts/speech';
import {historyText,summarizeHistory,type HistoryPort,type HistoryRecord,type HistoryCursor,type PlaybackState} from '../contracts/history';
export class BrowserSpeech implements SpeechPort {
 readonly mode='simulation' as const;
 value=initialSpeech();settings:RecordingPreferences={deviceId:null,historyEnabled:true,audioEnabled:true,retentionDays:30,revision:1};
 readonly listeners=new Set<(s:SpeechSnapshot)=>void>();readonly meters=new Set<(m:SpeechMeter)=>void>();readonly records:HistoryRecord[]=[];
 private timer:ReturnType<typeof setTimeout>|undefined;
 private active=false;private meterTimer:ReturnType<typeof setInterval>|undefined;
 async snapshot(){return structuredClone(this.value);}
 async subscribe(s:(v:SpeechSnapshot)=>void,m:(v:SpeechMeter)=>void){this.listeners.add(s);this.meters.add(m);return()=>{this.listeners.delete(s);this.meters.delete(m);};}
 async devices(){return[{id:'browser-simulated',name:'模拟麦克风（不采集声音）',isDefault:true}];}
 async preferences(){return {...this.settings};}
 async savePreferences(p:RecordingPreferences){if(p.revision!==this.settings.revision)throw new SpeechError('conflict');this.settings={...p,revision:p.revision+1};return this.preferences();}
 private emit(){this.value.sequence++;for(const fn of this.listeners)fn(structuredClone(this.value));}
 async start(){
  if(this.active)throw new SpeechError('busy');clearTimeout(this.timer);this.active=true;
  const prior=this.value;this.value={...initialSpeech(),sequence:prior.sequence+1,lifecycle:prior.lifecycle+1,taskId:crypto.randomUUID(),source:'trial',attemptId:crypto.randomUUID(),recordId:this.settings.historyEnabled?crypto.randomUUID():null,modelName:'模拟语音模型',deviceName:'模拟麦克风',phase:'connecting'};this.emit();
  this.timer=setTimeout(()=>{this.value.phase='recording';this.value.lifecycle++;this.value.text='这是浏览器模拟文字。';this.value.durationMs=1000;this.emit();let frame=0;this.meterTimer=setInterval(()=>{if(this.value.phase!=='recording')return;frame++;const energies=Array.from({length:15},(_,i)=>.04*(1+Math.sin(frame*.28+i*.6)));const meter:SpeechMeter={taskId:this.value.taskId!,sequence:frame,captureSequence:frame,level:energies.reduce((a,b)=>a+b,0)/15,energies,bars:energies.map(v=>Math.tanh(v*12)),durationMs:1000+frame*50};for(const fn of this.meters)fn(meter);},50);},250);return this.snapshot();
 }
 async control(id:string,action:'finish'|'cancel'){
  if(!this.active||id!==this.value.taskId)throw new SpeechError('staleSession');
  if(action==='finish'&&this.value.phase!=='recording')throw new SpeechError('invalid');clearTimeout(this.timer);clearInterval(this.meterTimer);
  if(action==='cancel'){this.active=false;this.value.phase='cancelled';this.value.text='';this.value.recordId=null;this.value.lifecycle++;this.emit();return;}
  if(this.value.phase!=='recording')throw new SpeechError('invalid');this.value.phase='processing';this.value.lifecycle++;this.emit();
  this.timer=setTimeout(()=>{this.active=false;this.value.phase='completed';this.value.text='这是浏览器模拟文字，未访问麦克风或语音服务。';this.value.lifecycle++;this.value.saved=this.settings.historyEnabled;
   if(this.value.recordId){const time=Date.now();this.records.unshift({id:this.value.recordId,createdAt:time,endedAt:time,durationMs:1000,status:'success',currentAttempt:this.value.attemptId,revision:1,audio:null,attempts:[{id:this.value.attemptId!,recordId:this.value.recordId,kind:'live',configRevision:1,configId:'simulation',configName:'模拟配置',modelId:'simulation',serviceHost:'simulation',createdAt:time,endedAt:time,status:'success',text:this.value.text,errorCode:null,elapsedMs:250}]});}
   this.emit();const taskId=this.value.taskId;this.timer=setTimeout(()=>{if(!this.active&&this.value.taskId===taskId){this.value.phase='idle';this.value.lifecycle++;this.emit();}},1000);},250);
 }
 async retryCurrent():Promise<SpeechSnapshot>{throw new SpeechError('fileUnavailable');}
 async copy(){/* Simulated feedback; no system clipboard access. */}
}
export class BrowserHistory implements HistoryPort {
 readonly mode='simulation' as const;private playbackState:PlaybackState={resourceId:null,playing:false,positionMs:0,durationMs:0};
 constructor(private speech:BrowserSpeech){}
 async status(){return {state:'ready'};}
 async browse(search:string,cursor:HistoryCursor|null=null,pageSize=50){
  if(!Number.isInteger(pageSize)||pageSize<1||pageSize>50)throw new SpeechError('invalid');
  if([...search].length>256||[...search].some(c=>c.codePointAt(0)!<32||(c.codePointAt(0)!>=127&&c.codePointAt(0)!<=159)))throw new SpeechError('invalid');
  const all=this.speech.records.map(r=>summarizeHistory(r,search)).filter(r=>r!==null).sort((a,b)=>b.createdAt-a.createdAt||(a.id<b.id?-1:1));
  const remaining=all.filter(r=>!cursor||r.createdAt<cursor.createdAt||r.createdAt===cursor.createdAt&&r.id>cursor.id);
  const records=remaining.slice(0,pageSize);const last=records.at(-1);return {records,total:all.length,nextCursor:remaining.length>pageSize&&last?{createdAt:last.createdAt,id:last.id}:null};
 }
 async reread(search:string,ids:string[]){
  if(ids.length>20||new Set(ids).size!==ids.length)throw new SpeechError('invalid');
  const all=this.speech.records.map(r=>summarizeHistory(r,search)).filter(r=>r!==null);
  return {records:all.filter(r=>ids.includes(r.id)),total:all.length,nextCursor:null};
 }
 async list(search:string,page:number){const records=this.speech.records.filter(r=>historyText(r).toLowerCase().includes(search.trim().toLowerCase()));const actual=Math.max(1,Math.min(page,Math.ceil(records.length/50)||1));return {records:structuredClone(records.slice((actual-1)*50,actual*50)),total:records.length,page:actual,pageSize:50 as const};}
 async detail(id:string){const r=this.speech.records.find(r=>r.id===id);if(!r)throw new SpeechError('notFound');return structuredClone(r);}
 async usage(){return {characters:this.speech.records.reduce((n,r)=>n+[...historyText(r)].filter(c=>!c.match(/\s/u)).length,0),durationMs:this.speech.records.reduce((n,r)=>n+r.durationMs,0),count:this.speech.records.length};}
 async remove(id:string,revision:number){const r=await this.detail(id);if(r.revision!==revision)throw new SpeechError('conflict');this.speech.records.splice(this.speech.records.findIndex(r=>r.id===id),1);}
 async retry():Promise<SpeechSnapshot>{throw new SpeechError('fileUnavailable');}
 async play():Promise<PlaybackState>{throw new SpeechError('fileUnavailable');}
 async playback(action:'pause'|'resume'|'seek'|'stop'|'status',ms?:number){if(action==='stop')this.playbackState={resourceId:null,playing:false,positionMs:0,durationMs:0};if(action==='seek')this.playbackState.positionMs=ms??0;return {...this.playbackState};}
}
