import {SpeechError,type SpeechSnapshot} from './speech';
export interface RecognitionAttempt {id:string;recordId:string;kind:'live'|'retry';configId:string;configName:string;configRevision:number;modelId:string;serviceHost:string;createdAt:number;endedAt:number|null;status:string;text:string;errorCode:string|null;elapsedMs:number}
export interface AudioResource {id:string;recordId:string;status:'prepared'|'ready'|'delete_pending'|'expire_pending'|'expired'|'missing'|'corrupt';bytes:number;expiresAt:number|null}
export interface HistoryRecord {id:string;createdAt:number;endedAt:number|null;durationMs:number;status:string;currentAttempt:string|null;revision:number;attempts:RecognitionAttempt[];audio:AudioResource|null}
export interface HistoryPage {records:HistoryRecord[];total:number;page:number;pageSize:50}
export interface HistoryCursor {createdAt:number;id:string}
export interface HistorySummary {id:string;createdAt:number;durationMs:number;status:string;revision:number;displayAttempt:string|null;preview:string;truncated:boolean;matchedOldVersion:boolean;latestAttempt:string|null;latestStatus:string|null;latestError:string|null;audio:AudioResource|null}
export interface HistoryFeed {records:HistorySummary[];total:number;nextCursor:HistoryCursor|null}
export interface Usage {characters:number;durationMs:number;count:number}
export interface PlaybackState {resourceId:string|null;playing:boolean;positionMs:number;durationMs:number}
export interface DeliveryReport {attemptId:string;state:string;error:string|null;updatedAt:number}
export interface HistoryPort {delivery?(id:string):Promise<DeliveryReport[]>;readonly mode:'native'|'simulation';status(retry?:boolean):Promise<{state:string}>;list(search:string,page:number):Promise<HistoryPage>;browse(search:string,cursor?:HistoryCursor|null):Promise<HistoryFeed>;detail(id:string):Promise<HistoryRecord>;usage():Promise<Usage>;remove(id:string,revision:number):Promise<void>;retry(id:string,configId:string,configRevision:number):Promise<SpeechSnapshot>;play(resourceId:string):Promise<PlaybackState>;playback(action:'pause'|'resume'|'seek'|'stop'|'status',positionMs?:number):Promise<PlaybackState>}
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
export function parseFeed(value:unknown):HistoryFeed {
 const p=obj(value);if(!Array.isArray(p.records)||p.records.length>50||!num(p.total))throw new SpeechError('protocol');
 const records=p.records.map(raw=>{const r=obj(raw);if(typeof r.id!=='string'||!num(r.createdAt)||!num(r.durationMs)||!num(r.revision)||r.revision<1||typeof r.status!=='string'||typeof r.preview!=='string'||[...r.preview].length>240||typeof r.truncated!=='boolean'||typeof r.matchedOldVersion!=='boolean')throw new SpeechError('protocol');
  for(const k of ['displayAttempt','latestAttempt','latestStatus','latestError'])if(r[k]!==null&&typeof r[k]!=='string')throw new SpeechError('protocol');
  if(r.audio!==null){const a=obj(r.audio);if(typeof a.id!=='string'||typeof a.recordId!=='string'||!num(a.bytes)||!(a.expiresAt===null||num(a.expiresAt))||!['prepared','ready','delete_pending','expire_pending','expired','missing','corrupt'].includes(String(a.status)))throw new SpeechError('protocol');}
  return r as unknown as HistorySummary;});
 let nextCursor:HistoryCursor|null=null;if(p.nextCursor!==null){const c=obj(p.nextCursor);if(!num(c.createdAt)||typeof c.id!=='string')throw new SpeechError('protocol');nextCursor=c as unknown as HistoryCursor;}
 return {records,total:p.total,nextCursor};
}
export function summarizeHistory(r:HistoryRecord,search=''):HistorySummary|null {
 const key=(v:string)=>v.trim().normalize('NFC').toLowerCase().normalize('NFC');const needle=key(search);
 const ordered=[...r.attempts].sort((a,b)=>b.createdAt-a.createdAt||(a.id<b.id?-1:a.id>b.id?1:0));
 const current=ordered.find(a=>a.id===r.currentAttempt);const matches=(a:RecognitionAttempt)=>!needle||key(a.text).includes(needle);
 const displayed=current&&matches(current)?current:ordered.find(matches);if(needle&&!displayed)return null;
 const text=[...(displayed?.text??'')];const latest=ordered[0];
 return {id:r.id,createdAt:r.createdAt,durationMs:r.durationMs,status:r.status,revision:r.revision,displayAttempt:displayed?.id??null,preview:text.slice(0,240).join(''),truncated:text.length>240,matchedOldVersion:!!needle&&displayed?.id!==r.currentAttempt,latestAttempt:latest?.id??null,latestStatus:latest?.status??null,latestError:latest?.errorCode??null,audio:r.audio?{...r.audio}:null};
}
export function parseUsage(v:unknown):Usage{const u=obj(v);if(!num(u.characters)||!num(u.durationMs)||!num(u.count))throw new SpeechError('protocol');return u as unknown as Usage;}
export function parseDeliveryReports(value:unknown):DeliveryReport[]{if(!Array.isArray(value))throw new SpeechError('protocol');return value.map(raw=>{const v=obj(raw);if(typeof v.attemptId!=='string'||!['pending','delivering','submitted','confirmed','failed','unconfirmed','copy'].includes(String(v.state))||!num(v.updatedAt)||!(v.error===null||typeof v.error==='string'))throw new SpeechError('protocol');return v as unknown as DeliveryReport;});}
export function parsePlayback(v:unknown):PlaybackState{const p=obj(v);if(!(p.resourceId===null||typeof p.resourceId==='string')||typeof p.playing!=='boolean'||!num(p.positionMs)||!num(p.durationMs))throw new SpeechError('protocol');return p as unknown as PlaybackState;}
