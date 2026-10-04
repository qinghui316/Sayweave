import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {speechError,parseSpeech,parseMeter,parseDevices,parsePreferences,type SpeechPort,type SpeechSnapshot,type SpeechMeter,type RecordingPreferences} from '../contracts/speech';
import {parseHistory,parseRecord,parseUsage,parsePlayback,type HistoryPort} from '../contracts/history';
import {BrowserSpeech,BrowserHistory} from './speech-browser';
async function call(command:string,args:Record<string,unknown>={}){try{return await invoke<unknown>(command,args);}catch(e){throw speechError(e);}}
class TauriSpeech implements SpeechPort {
 readonly mode='native' as const;
 async snapshot(){return parseSpeech(await call('speech_snapshot'));}
 async subscribe(s:(v:SpeechSnapshot)=>void,m:(v:SpeechMeter)=>void){
  const a=await listen<unknown>('speech:snapshot',e=>s(parseSpeech(e.payload)));
  try{const b=await listen<unknown>('speech:meter',e=>m(parseMeter(e.payload)));return()=>{a();b();};}catch(e){a();throw e;}
 }
 async devices(){return parseDevices(await call('speech_devices'));}
 async preferences(){return parsePreferences(await call('speech_preferences',{preferences:null}));}
 async savePreferences(preferences:RecordingPreferences){return parsePreferences(await call('speech_preferences',{preferences}));}
 async retryCurrent(){return parseSpeech(await call('speech_retry_current'));}
 async start(){return parseSpeech(await call('speech_start'));}
 async control(taskId:string,action:'finish'|'cancel'){await call('speech_control',{taskId,action});}
 async copy(text:string){await call('speech_copy',{text});}
}
class TauriHistory implements HistoryPort {
 readonly mode='native' as const;
 async status(retry=false){const s=await call('history_status',{retry});if(typeof s!=='object'||s===null||!('state'in s)||typeof s.state!=='string')throw speechError({code:'protocol'});return {state:s.state};}
 async list(search:string,page:number){return parseHistory(await call('history_list',{query:{search,page}}));}
 async detail(id:string){return parseRecord(await call('history_detail',{id}));}
 async usage(){return parseUsage(await call('history_usage'));}
 async remove(id:string,revision:number){await call('history_remove',{id,revision});}
 async retry(id:string,configId:string,configRevision:number){return parseSpeech(await call('history_retry',{id,configId,configRevision}));}
 async play(resourceId:string){return parsePlayback(await call('history_play',{resourceId,action:'play',positionMs:null}));}
 async playback(action:'pause'|'resume'|'seek'|'stop'|'status',positionMs?:number){return parsePlayback(await call('history_play',{resourceId:null,action,positionMs:positionMs??null}));}
}
export function createSpeechPorts():{speech:SpeechPort;history:HistoryPort}{if(isTauri())return {speech:new TauriSpeech(),history:new TauriHistory()};const speech=new BrowserSpeech();return {speech,history:new BrowserHistory(speech)};}
