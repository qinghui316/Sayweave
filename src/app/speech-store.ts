import {initialSpeech,type SpeechPort,type SpeechSnapshot,type SpeechMeter,type SpeechArchive,type SpeechMeterSource,type SpeechMeterFrame} from '../contracts/speech';
export class SpeechStore {
 private value=initialSpeech();private level=0;private meterSequence=0;private display={snapshot:this.value,level:0};private listeners=new Set<()=>void>();
 private trialDisplay={snapshot:initialSpeech(),level:0};private trialMeterSequence=0;private trialListeners=new Set<()=>void>();
 private archiveFailure:SpeechArchive|null=null;private archiveListeners=new Set<()=>void>();
 private archiveRevisions=new Map<string,number>();
 private meterValue:SpeechMeterFrame|null=null;private meterListeners=new Set<()=>void>();
 private trialMeterValue:SpeechMeterFrame|null=null;private trialMeterListeners=new Set<()=>void>();
 readonly liveMeter:SpeechMeterSource={getSnapshot:()=>this.meterValue,subscribe:fn=>{this.meterListeners.add(fn);return()=>{this.meterListeners.delete(fn);};}};
 readonly trialMeter:SpeechMeterSource={getSnapshot:()=>this.trialMeterValue,subscribe:fn=>{this.trialMeterListeners.add(fn);return()=>{this.trialMeterListeners.delete(fn);};}};
 private clearMeter(){this.meterValue=null;this.trialMeterValue=null;for(const fn of this.meterListeners)fn();for(const fn of this.trialMeterListeners)fn();}
 constructor(readonly port:SpeechPort){}
 subscribe=(fn:()=>void)=>{this.listeners.add(fn);return()=>{this.listeners.delete(fn);};};
 getSnapshot=()=>this.display;
 subscribeTrial=(fn:()=>void)=>{this.trialListeners.add(fn);return()=>{this.trialListeners.delete(fn);};};
 getTrialSnapshot=()=>this.trialDisplay;
 subscribeArchive=(fn:()=>void)=>{this.archiveListeners.add(fn);return()=>{this.archiveListeners.delete(fn);};};
 getArchiveFailure=()=>this.archiveFailure;
 dismissArchiveFailure=()=>{this.archiveFailure=null;for(const fn of this.archiveListeners)fn();};
 private emit(){this.display={snapshot:this.value,level:this.level};for(const fn of this.listeners)fn();}
 private emitTrial(){for(const fn of this.trialListeners)fn();}
 accept=(s:SpeechSnapshot)=>{
  const globalNew=s.sequence>this.value.sequence;
  if(globalNew){if(s.taskId!==this.value.taskId){this.meterSequence=0;this.level=0;this.clearMeter();}this.value=s;if(s.phase!=='recording'){this.level=0;this.clearMeter();}}
  let trialNew=false;
  if(s.source==='trial'&&s.sequence>this.trialDisplay.snapshot.sequence){
   const sameTask=s.taskId===this.trialDisplay.snapshot.taskId;if(!sameTask)this.trialMeterSequence=0;
   this.trialDisplay={snapshot:s,level:sameTask&&s.phase==='recording'?this.trialDisplay.level:0};trialNew=true;
  }
  if(this.value.taskId!==this.trialDisplay.snapshot.taskId&&this.trialDisplay.level!==0){this.trialDisplay={...this.trialDisplay,level:0};trialNew=true;}
  if(globalNew)this.emit();if(trialNew)this.emitTrial();
 };
 archived=(result:SpeechArchive)=>{
  const matches=(snapshot:SpeechSnapshot)=>result.taskId===snapshot.taskId&&(result.attemptId===undefined||result.attemptId===snapshot.attemptId);
  if(!matches(this.value)&&!matches(this.trialDisplay.snapshot))return;
  const revision=result.revision??0;if(revision<=(this.archiveRevisions.get(result.taskId)??-1))return;
  this.archiveRevisions.set(result.taskId,revision);
  for(const id of this.archiveRevisions.keys())if(id!==this.value.taskId&&id!==this.trialDisplay.snapshot.taskId)this.archiveRevisions.delete(id);
  if(matches(this.value)){this.value={...this.value,saved:result.saved,saveError:result.saveError};this.emit();}
  if(matches(this.trialDisplay.snapshot)){this.trialDisplay={...this.trialDisplay,snapshot:{...this.trialDisplay.snapshot,saved:result.saved,saveError:result.saveError}};this.emitTrial();}
 };
 meter=(m:SpeechMeter)=>{
  if(m.taskId!==this.value.taskId||m.sequence<=this.meterSequence||this.value.phase!=='recording')return;
  this.meterSequence=m.sequence;
  if(this.meterValue&&m.captureSequence<=this.meterValue.captureSequence)return;
  this.meterValue={...m,receivedAt:performance.now()};this.level=Math.min(m.level,1);
  for(const fn of this.meterListeners)fn();
  const durationMs=Math.max(this.value.durationMs,m.durationMs);
  const secondChanged=Math.floor(durationMs/1000)!==Math.floor(this.value.durationMs/1000);
  this.value={...this.value,durationMs};
  if(m.taskId===this.trialDisplay.snapshot.taskId&&m.sequence>this.trialMeterSequence){
   this.trialMeterSequence=m.sequence;this.trialMeterValue=this.meterValue;
   for(const fn of this.trialMeterListeners)fn();
   const snapshot=this.trialDisplay.snapshot;
   this.trialDisplay={snapshot:{...snapshot,durationMs:Math.max(snapshot.durationMs,m.durationMs)},level:this.level};
   if(secondChanged)this.emitTrial();
  }
  if(secondChanged)this.emit();
 };
 async connect(){const release=await this.port.subscribe(s=>this.accept(s),m=>this.meter(m),this.archived);try{this.accept(await this.port.snapshot());return release;}catch(e){release();throw e;}}
 async resync(){this.accept(await this.port.snapshot());}
}
