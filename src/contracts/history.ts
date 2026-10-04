import {SpeechError,type SpeechSnapshot} from './speech';
export interface RecognitionAttempt {id:string;recordId:string;kind:'live'|'retry';configId:string;configName:string;configRevision:number;modelId:string;serviceHost:string;createdAt:number;endedAt:number|null;status:string;text:string;errorCode:string|null;elapsedMs:number}
export interface AudioResource {id:string;recordId:string;status:'prepared'|'ready'|'delete_pending'|'expire_pending'|'expired'|'missing'|'corrupt';bytes:number;expiresAt:number|null}
export interface HistoryRecord {id:string;createdAt:number;endedAt:number|null;durationMs:number;status:string;currentAttempt:string|null;revision:number;attempts:RecognitionAttempt[];audio:AudioResource|null}
export interface HistoryPage {records:HistoryRecord[];total:number;page:number;pageSize:50}
export interface Usage {characters:number;durationMs:number;count:number}
export interface PlaybackState {resourceId:string|null;playing:boolean;positionMs:number;durationMs:number}
export interface HistoryPort {readonly mode:'native'|'simulation';status(retry?:boolean):Promise<{state:string}>;list(search:string,page:number):Promise<HistoryPage>;detail(id:string):Promise<HistoryRecord>;usage():Promise<Usage>;remove(id:string,revision:number):Promise<void>;retry(id:string,configId:string,configRevision:number):Promise<SpeechSnapshot>;play(resourceId:string):Promise<PlaybackState>;playback(action:'pause'|'resume'|'seek'|'stop'|'status',positionMs?:number):Promise<PlaybackState>}
export const historyText=(r:HistoryRecord)=>r.attempts.find(a=>a.id===r.currentAttempt)?.text??r.attempts[0]?.text??'';
const obj=(v:unknown):Record<string,unknown>=>{if(typeof v!=='object'||v===null||Array.isArray(v))throw new SpeechError('protocol');return v as Record<string,unknown>;};
const num=(v:unknown):v is number=>typeof v==='number'&&Number.isSafeInteger(v)&&v>=0;
export function parseRecord(value:unknown):HistoryRecord{
 const r=obj(value);if(typeof r.id!=='string'||!num(r.createdAt)||!num(r.durationMs)||typeof r.status!=='string'||!num(r.revision)||!Array.isArray(r.attempts)||!(r.currentAttempt===null||typeof r.currentAttempt==='string')||!(r.endedAt===null||num(r.endedAt)))throw new SpeechError('protocol');
 for(const raw of r.attempts){const a=obj(raw);for(const key of ['id','recordId','kind','configId','configName','modelId','serviceHost','status','text'])if(typeof a[key]!=='string')throw new SpeechError('protocol');if(!num(a.configRevision)||a.configRevision<1||!num(a.createdAt)||!num(a.elapsedMs)||!(a.endedAt===null||num(a.endedAt))||!(a.errorCode===null||typeof a.errorCode==='string'))throw new SpeechError('protocol');}
 if(r.audio!==null){const a=obj(r.audio);if(typeof a.id!=='string'||typeof a.recordId!=='string'||!['prepared','ready','delete_pending','expire_pending','expired','missing','corrupt'].includes(String(a.status))||!num(a.bytes)||!(a.expiresAt===null||num(a.expiresAt)))throw new SpeechError('protocol');}
 return r as unknown as HistoryRecord;
}
export function parseHistory(value:unknown):HistoryPage{const p=obj(value);if(!Array.isArray(p.records)||!num(p.total)||!num(p.page)||p.page<1||p.pageSize!==50)throw new SpeechError('protocol');return {records:p.records.map(parseRecord),total:p.total,page:p.page,pageSize:50};}
export function parseUsage(v:unknown):Usage{const u=obj(v);if(!num(u.characters)||!num(u.durationMs)||!num(u.count))throw new SpeechError('protocol');return u as unknown as Usage;}
export function parsePlayback(v:unknown):PlaybackState{const p=obj(v);if(!(p.resourceId===null||typeof p.resourceId==='string')||typeof p.playing!=='boolean'||!num(p.positionMs)||!num(p.durationMs))throw new SpeechError('protocol');return p as unknown as PlaybackState;}
