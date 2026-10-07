import {initialSpeech,type SpeechPort,type SpeechSnapshot,type SpeechMeter,type SpeechArchive} from '../contracts/speech';
export class SpeechStore {
 private value=initialSpeech();private level=0;private meterSequence=0;private display={snapshot:this.value,level:0};private listeners=new Set<()=>void>();
 private trialDisplay={snapshot:initialSpeech(),level:0};private trialMeterSequence=0;private trialListeners=new Set<()=>void>();
 private archiveFailure:SpeechArchive|null=null;private archiveListeners=new Set<()=>void>();
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
  if(globalNew){if(s.taskId!==this.value.taskId){this.meterSequence=0;this.level=0;}this.value=s;if(s.phase!=='recording')this.level=0;}
  let trialNew=false;
  if(s.source==='trial'&&s.sequence>this.trialDisplay.snapshot.sequence){
   const sameTask=s.taskId===this.trialDisplay.snapshot.taskId;if(!sameTask)this.trialMeterSequence=0;
   this.trialDisplay={snapshot:s,level:sameTask&&s.phase==='recording'?this.trialDisplay.level:0};trialNew=true;
  }
  if(this.value.taskId!==this.trialDisplay.snapshot.taskId&&this.trialDisplay.level!==0){this.trialDisplay={...this.trialDisplay,level:0};trialNew=true;}
  if(globalNew)this.emit();if(trialNew)this.emitTrial();
 };
 archived=(result:SpeechArchive)=>{
  if(result.saveError){this.archiveFailure=result;for(const fn of this.archiveListeners)fn();}
  if(result.taskId===this.value.taskId){this.value={...this.value,saved:result.saved,saveError:result.saveError};this.emit();}
  if(result.taskId===this.trialDisplay.snapshot.taskId){this.trialDisplay={...this.trialDisplay,snapshot:{...this.trialDisplay.snapshot,saved:result.saved,saveError:result.saveError}};this.emitTrial();}
 };
 meter=(m:SpeechMeter)=>{
  if(m.taskId!==this.value.taskId||m.sequence<=this.meterSequence)return;
  this.meterSequence=m.sequence;this.level=this.value.phase==='recording'?Math.min(m.level,1):0;
  this.value={...this.value,durationMs:Math.max(this.value.durationMs,m.durationMs)};
  if(m.taskId===this.trialDisplay.snapshot.taskId&&m.sequence>this.trialMeterSequence){
   this.trialMeterSequence=m.sequence;const snapshot=this.trialDisplay.snapshot;
   this.trialDisplay={snapshot:{...snapshot,durationMs:Math.max(snapshot.durationMs,m.durationMs)},level:snapshot.phase==='recording'?this.level:0};
  }
  this.emit();if(m.taskId===this.trialDisplay.snapshot.taskId)this.emitTrial();
 };
 async connect(){const release=await this.port.subscribe(s=>this.accept(s),m=>this.meter(m),this.archived);try{this.accept(await this.port.snapshot());return release;}catch(e){release();throw e;}}
 async resync(){this.accept(await this.port.snapshot());}
}
