import type {SpeechMeterFrame} from '../contracts/speech';
/** Convert measured RMS to a readable level without moving the silence floor. */
export function waveformLevel(rms: number) {
  if (!Number.isFinite(rms) || rms <= .001) return 0;
  return Math.max(0, Math.min(1, (20 * Math.log10(rms) + 60) / 45));
}
/** Per-instance bounded gain and envelopes. Silence decoration requires fresh capture. */
export class WaveEnvelope {
  private task:string|null=null;
  private sequence=-1;
  private peak=.08;
  private targets=Array<number>(15).fill(0);
  private visible=Array<number>(15).fill(0);
  draw(frame:SpeechMeterFrame|null,now:number,dt:number,reduced:boolean,legacyLevel?:number){
    if(this.task!==(frame?.taskId??null)){
      this.task=frame?.taskId??null;this.sequence=-1;this.peak=.08;this.targets.fill(0);this.visible.fill(0);
    }
    if(frame&&frame.captureSequence!==this.sequence){
      this.sequence=frame.captureSequence;
      this.peak=Math.max(.012,Math.max(...frame.energies),this.peak*.96);
      const gain=Math.min(12,.24/this.peak);
      this.targets=frame.energies.map(rms=>rms<=.001?0:Math.min(1,Math.tanh((rms-.001)*gain*3)));
    }
    const age=frame?now-frame.receivedAt:Infinity;
    const fresh=age<=250;
    return this.visible.map((previous,i)=>{
      const target=legacyLevel!==undefined&&!frame?waveformLevel(legacyLevel):fresh?(this.targets[i]??0):0;
      const duration=target>previous?50:180;
      const next=reduced?target:previous+(target-previous)*(1-Math.exp(-Math.max(0,dt)/duration));
      this.visible[i]=next;
      const breath=!reduced&&age<1000?.018*(1+Math.sin(now*2*Math.PI/1800+i*.45))*(1-next):0;
      return Math.min(1,.084+next*.916+breath);
    });
  }
}
