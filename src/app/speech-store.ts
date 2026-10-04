import {initialSpeech,type SpeechPort,type SpeechSnapshot,type SpeechMeter} from '../contracts/speech';
export class SpeechStore {
 private value=initialSpeech();private level=0;private meterSequence=0;private display={snapshot:this.value,level:0};private listeners=new Set<()=>void>();
 constructor(readonly port:SpeechPort){}
 subscribe=(fn:()=>void)=>{this.listeners.add(fn);return()=>{this.listeners.delete(fn);};};
 getSnapshot=()=>this.display;
 private emit(){this.display={snapshot:this.value,level:this.level};for(const fn of this.listeners)fn();}
 accept=(s:SpeechSnapshot)=>{if(s.sequence<=this.value.sequence)return;if(s.taskId!==this.value.taskId){this.meterSequence=0;this.level=0;}this.value=s;if(s.phase!=='recording')this.level=0;this.emit();};
 meter=(m:SpeechMeter)=>{if(m.taskId!==this.value.taskId||m.sequence<=this.meterSequence)return;this.meterSequence=m.sequence;this.level=this.value.phase==='recording'?Math.min(m.level,1):0;this.value={...this.value,durationMs:Math.max(this.value.durationMs,m.durationMs)};this.emit();};
 async connect(){const release=await this.port.subscribe(s=>this.accept(s),m=>this.meter(m));try{this.accept(await this.port.snapshot());return release;}catch(e){release();throw e;}}
 async resync(){this.accept(await this.port.snapshot());}
}
