import {SpeechError,type SpeechSnapshot} from './speech';
export interface RecognitionAttempt {id:string;recordId:string;kind:'live'|'retry';configId:string;configName:string;configRevision:number;modelId:string;serviceHost:string;createdAt:number;endedAt:number|null;status:string;text:string;errorCode:string|null;elapsedMs:number}
export interface AudioResource {id:string;recordId:string;status:'prepared'|'ready'|'delete_pending'|'expire_pending'|'expired'|'missing'|'corrupt';bytes:number;expiresAt:number|null}
export interface ProcessingUnit {id:string;runId:string;turn:number;sourceStart:number;sourceEnd:number;sourceRefs:string;sourceText:string;candidateText:string|null;generationState:string;applicationState:string;sourceCommittedEnd:number|null;errorCode:string|null;promptTokens:number|null;completionTokens:number|null;elapsedMs:number}
export interface ProcessingVersion {id:string;attemptId:string;configName:string;modelId:string;state:string;text:string;revision:number;errorCode:string|null;units:ProcessingUnit[]}
export interface TextReference {kind:'raw'|'processed';id:string;attemptId:string}
export interface HistoryRecord {id:string;createdAt:number;endedAt:number|null;durationMs:number;status:string;currentAttempt:string|null;revision:number;attempts:RecognitionAttempt[];audio:AudioResource|null;processing?:ProcessingVersion[]}
export interface HistoryPage {records:HistoryRecord[];total:number;page:number;pageSize:50}
export interface HistoryCursor {createdAt:number;id:string}
export interface HistorySummary {id:string;createdAt:number;durationMs:number;status:string;revision:number;displayAttempt:string|null;displayReference?:TextReference|null;preview:string;truncated:boolean;matchedOldVersion:boolean;latestAttempt:string|null;latestStatus:string|null;latestError:string|null;audio:AudioResource|null}
export interface HistoryFeed {records:HistorySummary[];total:number;nextCursor:HistoryCursor|null}
export interface Usage {characters:number;durationMs:number;count:number}
export interface PlaybackState {resourceId:string|null;playing:boolean;positionMs:number;durationMs:number}
export interface DeliveryReport {attemptId:string;state:string;error:string|null;updatedAt:number}
export interface HistoryPort {delivery?(id:string):Promise<DeliveryReport[]>;readonly mode:'native'|'simulation';status(retry?:boolean):Promise<{state:string}>;list(search:string,page:number):Promise<HistoryPage>;browse(search:string,cursor?:HistoryCursor|null,pageSize?:number):Promise<HistoryFeed>;reread(search:string,ids:string[]):Promise<HistoryFeed>;detail(id:string):Promise<HistoryRecord>;usage():Promise<Usage>;remove(id:string,revision:number):Promise<void>;retry(id:string,configId:string,configRevision:number):Promise<SpeechSnapshot>;play(resourceId:string):Promise<PlaybackState>;playback(action:'pause'|'resume'|'seek'|'stop'|'status',positionMs?:number):Promise<PlaybackState>}
export const historySelection=(r:HistorySummary)=>r.displayReference?.kind==='processed'?'processed:'+r.displayReference.id:r.displayAttempt;
export function historyVersion(r:HistoryRecord,id:string|null){const processed=id?.startsWith('processed:')?r.processing?.find(p=>p.id===id.slice(10)):undefined;const attempt=r.attempts.find(a=>a.id===(processed?.attemptId??id));return {text:processed?.text??attempt?.text??'',attempt,processed};}
export const historyText=(r:HistoryRecord)=>r.processing?.find(p=>p.attemptId===r.currentAttempt&&p.state!=='running'&&p.text)?.text??r.attempts.find(a=>a.id===r.currentAttempt)?.text??r.attempts[0]?.text??'';
const obj=(v:unknown):Record<string,unknown>=>{if(typeof v!=='object'||v===null||Array.isArray(v))throw new SpeechError('protocol');return v as Record<string,unknown>;};
const num=(v:unknown):v is number=>typeof v==='number'&&Number.isSafeInteger(v)&&v>=0;
function processingUnit(value:unknown,runId:string){
 const u=obj(value);for(const key of ['id','runId','sourceRefs','sourceText'])if(typeof u[key]!=='string')throw new SpeechError('protocol');
 if(u.runId!==runId||!num(u.turn)||u.turn<1||!num(u.sourceStart)||!num(u.sourceEnd)||u.sourceEnd<=u.sourceStart||!num(u.elapsedMs)||!['pending','success','failed','skipped','interrupted'].includes(String(u.generationState))||!['pending','applied','handedOff','notApplied','uncertain','cancelled'].includes(String(u.applicationState)))throw new SpeechError('protocol');
 for(const key of ['candidateText','errorCode'])if(u[key]!==null&&typeof u[key]!=='string')throw new SpeechError('protocol');
 for(const key of ['promptTokens','completionTokens','sourceCommittedEnd'])if(u[key]!==null&&!num(u[key]))throw new SpeechError('protocol');
 if(typeof u.sourceCommittedEnd==='number'&&(u.sourceCommittedEnd<u.sourceStart||u.sourceCommittedEnd>u.sourceEnd))throw new SpeechError('protocol');
 let refs:unknown;try{refs=JSON.parse(String(u.sourceRefs));}catch{throw new SpeechError('protocol');}if(!Array.isArray(refs))throw new SpeechError('protocol');
 for(const raw of refs){const ref=obj(raw),range=obj(ref.chars);if(!num(ref.segment)||!num(ref.revision)||!num(range.start)||!num(range.end)||range.end<range.start)throw new SpeechError('protocol');}
}
export function parseRecord(value:unknown):HistoryRecord{
 const r=obj(value);if(typeof r.id!=='string'||!num(r.createdAt)||!num(r.durationMs)||typeof r.status!=='string'||!num(r.revision)||!Array.isArray(r.attempts)||!(r.currentAttempt===null||typeof r.currentAttempt==='string')||!(r.endedAt===null||num(r.endedAt)))throw new SpeechError('protocol');
 for(const raw of r.attempts){const a=obj(raw);for(const key of ['id','recordId','kind','configId','configName','modelId','serviceHost','status','text'])if(typeof a[key]!=='string')throw new SpeechError('protocol');if(!num(a.configRevision)||a.configRevision<1||!num(a.createdAt)||!num(a.elapsedMs)||!(a.endedAt===null||num(a.endedAt))||!(a.errorCode===null||typeof a.errorCode==='string'))throw new SpeechError('protocol');}
 if(r.processing!==undefined){if(!Array.isArray(r.processing))throw new SpeechError('protocol');for(const raw of r.processing){const p=obj(raw);for(const key of ['id','attemptId','configName','modelId','state','text'])if(typeof p[key]!=='string')throw new SpeechError('protocol');if(!num(p.revision)||p.revision<1||!Array.isArray(p.units)||!['running','success','partial','failed','cancelled','interrupted'].includes(String(p.state))||p.errorCode!==null&&typeof p.errorCode!=='string'||!r.attempts.some(a=>obj(a).id===p.attemptId))throw new SpeechError('protocol');for(const unit of p.units)processingUnit(unit,String(p.id));}}
 if(r.audio!==null){const a=obj(r.audio);if(typeof a.id!=='string'||typeof a.recordId!=='string'||!['prepared','ready','delete_pending','expire_pending','expired','missing','corrupt'].includes(String(a.status))||!num(a.bytes)||!(a.expiresAt===null||num(a.expiresAt)))throw new SpeechError('protocol');}
 return r as unknown as HistoryRecord;
}
export function parseHistory(value:unknown):HistoryPage{const p=obj(value);if(!Array.isArray(p.records)||!num(p.total)||!num(p.page)||p.page<1||p.pageSize!==50)throw new SpeechError('protocol');return {records:p.records.map(parseRecord),total:p.total,page:p.page,pageSize:50};}
export function parseFeed(value:unknown):HistoryFeed {
 const p=obj(value);if(!Array.isArray(p.records)||p.records.length>50||!num(p.total))throw new SpeechError('protocol');
 const records=p.records.map(raw=>{const r=obj(raw);if(typeof r.id!=='string'||!num(r.createdAt)||!num(r.durationMs)||!num(r.revision)||r.revision<1||typeof r.status!=='string'||typeof r.preview!=='string'||[...r.preview].length>240||typeof r.truncated!=='boolean'||typeof r.matchedOldVersion!=='boolean')throw new SpeechError('protocol');
  for(const k of ['displayAttempt','latestAttempt','latestStatus','latestError'])if(r[k]!==null&&typeof r[k]!=='string')throw new SpeechError('protocol');
  if(r.displayReference!==undefined&&r.displayReference!==null){const ref=obj(r.displayReference);if(!['raw','processed'].includes(String(ref.kind))||typeof ref.id!=='string'||typeof ref.attemptId!=='string'||ref.attemptId!==r.displayAttempt)throw new SpeechError('protocol');}
  if(r.audio!==null){const a=obj(r.audio);if(typeof a.id!=='string'||typeof a.recordId!=='string'||!num(a.bytes)||!(a.expiresAt===null||num(a.expiresAt))||!['prepared','ready','delete_pending','expire_pending','expired','missing','corrupt'].includes(String(a.status)))throw new SpeechError('protocol');}
  return r as unknown as HistorySummary;});
 let nextCursor:HistoryCursor|null=null;if(p.nextCursor!==null){const c=obj(p.nextCursor);if(!num(c.createdAt)||typeof c.id!=='string')throw new SpeechError('protocol');nextCursor=c as unknown as HistoryCursor;}
 return {records,total:p.total,nextCursor};
}
export function summarizeHistory(r:HistoryRecord,search=''):HistorySummary|null {
 const key=(v:string)=>v.trim().normalize('NFC').toLowerCase().normalize('NFC'),needle=key(search);
 const ordered=[...r.attempts].sort((a,b)=>Number(b.id===r.currentAttempt)-Number(a.id===r.currentAttempt)||b.createdAt-a.createdAt||(a.id<b.id?-1:a.id>b.id?1:0));
 let selected:{attempt:RecognitionAttempt;text:string;reference:TextReference}|null=null;
 for(const attempt of ordered){
  const processed=r.processing?.find(p=>p.attemptId===attempt.id&&p.state!=='running'&&p.text&&(!needle||key(p.text).includes(needle)));
  if(processed){selected={attempt,text:processed.text,reference:{kind:'processed',id:processed.id,attemptId:attempt.id}};break;}
  if(!needle||key(attempt.text).includes(needle)){selected={attempt,text:attempt.text,reference:{kind:'raw',id:attempt.id,attemptId:attempt.id}};break;}
 }
 if(needle&&!selected)return null;
 const text=[...(selected?.text??'')],latest=[...r.attempts].sort((a,b)=>b.createdAt-a.createdAt||(a.id<b.id?-1:1))[0];
 return {id:r.id,createdAt:r.createdAt,durationMs:r.durationMs,status:r.status,revision:r.revision,displayAttempt:selected?.attempt.id??null,displayReference:selected?.reference??null,preview:text.slice(0,240).join(''),truncated:text.length>240,matchedOldVersion:!!needle&&selected?.attempt.id!==r.currentAttempt,latestAttempt:latest?.id??null,latestStatus:latest?.status??null,latestError:latest?.errorCode??null,audio:r.audio?{...r.audio}:null};
}
export function parseUsage(v:unknown):Usage{const u=obj(v);if(!num(u.characters)||!num(u.durationMs)||!num(u.count))throw new SpeechError('protocol');return u as unknown as Usage;}
export function parseDeliveryReports(value:unknown):DeliveryReport[]{if(!Array.isArray(value))throw new SpeechError('protocol');return value.map(raw=>{const v=obj(raw);if(typeof v.attemptId!=='string'||!['pending','delivering','submitted','confirmed','failed','unconfirmed','copy'].includes(String(v.state))||!num(v.updatedAt)||!(v.error===null||typeof v.error==='string'))throw new SpeechError('protocol');return v as unknown as DeliveryReport;});}
export function parsePlayback(v:unknown):PlaybackState{const p=obj(v);if(!(p.resourceId===null||typeof p.resourceId==='string')||typeof p.playing!=='boolean'||!num(p.positionMs)||!num(p.durationMs))throw new SpeechError('protocol');return p as unknown as PlaybackState;}
