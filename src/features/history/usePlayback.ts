import {useCallback,useEffect,useRef,useState} from 'react';
import type {HistoryPort,PlaybackState} from '../../contracts/history';
import {SpeechError} from '../../contracts/speech';
const empty:PlaybackState={resourceId:null,playing:false,positionMs:0,durationMs:0};
// Keep commands ordered across route unmount/remount, including a slow native play.
const operations=new WeakMap<HistoryPort,Promise<unknown>>();
function ordered<T>(port:HistoryPort,operation:()=>Promise<T>):Promise<T>{const prior=operations.get(port)??Promise.resolve();const next=prior.catch(()=>{}).then(operation);operations.set(port,next);void next.finally(()=>{if(operations.get(port)===next)operations.delete(port);}).catch(()=>{});return next;}
export function useHistoryPlayback(port:HistoryPort,visible:boolean){
 const [state,setState]=useState(empty),[busy,setBusy]=useState(false);const alive=useRef(false),generation=useRef(0),pending=useRef(false);
 const flight=useRef<Promise<void>|null>(null);
 useEffect(()=>{alive.current=true;generation.current++;return()=>{alive.current=false;void ordered(port,()=>port.playback('stop')).catch(()=>{});};},[port]);
 useEffect(()=>{
  if(!visible||pending.current)return;const token=generation.current;let cancelled=false;
  void port.playback('status').then(v=>{if(!cancelled&&alive.current&&token===generation.current)setState(v);}).catch(()=>{});
  return()=>{cancelled=true;};
 },[port,visible]);
 useEffect(()=>{
  if(!visible||!state.playing)return;let cancelled=false;let timer:ReturnType<typeof setTimeout>;
  async function poll(){const token=generation.current;if(!pending.current)try{const value=await port.playback('status');if(!cancelled&&alive.current&&token===generation.current)setState(value);}catch{if(!cancelled&&token===generation.current)setState(empty);}if(!cancelled)timer=setTimeout(()=>void poll(),250);}
  timer=setTimeout(()=>void poll(),250);return()=>{cancelled=true;clearTimeout(timer);};
 },[port,visible,state.playing]);
 const control=useCallback(async(action:'play'|'pause'|'resume'|'seek'|'stop',resourceId?:string,positionMs?:number)=>{
  if(!alive.current)throw new SpeechError('cancelled');
  if(pending.current){if(action==='stop')await flight.current;else return;}if(!alive.current)throw new SpeechError('cancelled');pending.current=true;const token=++generation.current;setBusy(true);
  let release!:()=>void;const currentFlight=new Promise<void>(resolve=>{release=resolve;});flight.current=currentFlight;
  try{const value=await ordered(port,()=>action==='play'?port.play(resourceId!):port.playback(action,positionMs));if(alive.current&&token===generation.current)setState(value);}
  finally{release();if(flight.current===currentFlight){pending.current=false;flight.current=null;if(alive.current)setBusy(false);}}
 },[port]);
 return {state,busy,control};
}
