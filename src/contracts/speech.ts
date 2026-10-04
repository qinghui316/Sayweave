import type {Phase} from './runtime';
export const speechErrorCodes=['invalid','unavailable','preparing','stopping','busy','notFound','conflict','staleSession','forbidden','notConfigured','unsupported','credentialMissing','credentialUnavailable','deviceMissing','deviceAccess','deviceFormat','bufferOverflow','auth','resourceUnavailable','rateLimit','timeout','network','protocol','service','responseTooLarge','emptyText','cancelled','saveFailed','fileUnavailable','deleteFailed','playbackFailed','copyFailed','interrupted'] as const;
export type SpeechErrorCode=typeof speechErrorCodes[number];
export class SpeechError extends Error {constructor(readonly code:SpeechErrorCode){super(code);}}
export function speechError(e:unknown):SpeechError {if(e instanceof SpeechError)return e;if(typeof e==='object'&&e!==null&&'code'in e&&speechErrorCodes.includes(e.code as SpeechErrorCode))return new SpeechError(e.code as SpeechErrorCode);return new SpeechError('unavailable');}
export interface SpeechSegment {id:number;sequence:number;text:string;confirmed:boolean;beginMs:number|null;endMs:number|null}
export interface SpeechSnapshot {protocol:1;sequence:number;lifecycle:number;taskId:string|null;attemptId:string|null;recordId:string|null;phase:Phase;text:string;segments:SpeechSegment[];modelName:string|null;deviceName:string|null;durationMs:number;error:SpeechErrorCode|null;saveError:boolean;saved:boolean;retry:boolean;canRetry:boolean}
export interface SpeechMeter {taskId:string;sequence:number;level:number;durationMs:number}
export interface Microphone {id:string;name:string;isDefault:boolean}
export interface RecordingPreferences {deviceId:string|null;historyEnabled:boolean;audioEnabled:boolean;retentionDays:7|30|90|null;revision:number}
export interface SpeechPort {
 readonly mode:'native'|'simulation';snapshot():Promise<SpeechSnapshot>;
 subscribe(snapshot:(value:SpeechSnapshot)=>void,meter:(value:SpeechMeter)=>void):Promise<()=>void>;
 devices():Promise<Microphone[]>;preferences():Promise<RecordingPreferences>;savePreferences(value:RecordingPreferences):Promise<RecordingPreferences>;
 retryCurrent():Promise<SpeechSnapshot>;start():Promise<SpeechSnapshot>;control(taskId:string,action:'finish'|'cancel'):Promise<void>;copy(text:string):Promise<void>;
}
export const initialSpeech=():SpeechSnapshot=>({protocol:1,sequence:1,lifecycle:1,taskId:null,attemptId:null,recordId:null,phase:'idle',text:'',segments:[],modelName:null,deviceName:null,durationMs:0,error:null,saveError:false,saved:false,retry:false,canRetry:false});
const object=(v:unknown):Record<string,unknown>=>{if(typeof v!=='object'||v===null||Array.isArray(v))throw new SpeechError('protocol');return v as Record<string,unknown>;};
const count=(v:unknown):v is number=>typeof v==='number'&&Number.isSafeInteger(v)&&v>=0;
const nullableText=(v:unknown):v is string|null=>v===null||typeof v==='string';
export function parseSpeech(v:unknown):SpeechSnapshot {
 const s=object(v);if(s.protocol!==1||!count(s.sequence)||!count(s.lifecycle)||!['idle','connecting','recording','processing','completed','cancelled','failed'].includes(String(s.phase))||!nullableText(s.taskId)||!nullableText(s.attemptId)||!nullableText(s.recordId)||typeof s.text!=='string'||s.text.length>1024*1024||!Array.isArray(s.segments)||!nullableText(s.modelName)||!nullableText(s.deviceName)||!count(s.durationMs)||!nullableText(s.error)||typeof s.saveError!=='boolean'||typeof s.saved!=='boolean'||typeof s.retry!=='boolean'||typeof s.canRetry!=='boolean')throw new SpeechError('protocol');
 for(const raw of s.segments){const p=object(raw);if(!count(p.id)||!count(p.sequence)||typeof p.text!=='string'||typeof p.confirmed!=='boolean'||!(p.beginMs===null||count(p.beginMs))||!(p.endMs===null||count(p.endMs)))throw new SpeechError('protocol');}
 if(s.error!==null&&!speechErrorCodes.includes(s.error as SpeechErrorCode))throw new SpeechError('protocol');return s as unknown as SpeechSnapshot;
}
export function parseMeter(v:unknown):SpeechMeter{const s=object(v);if(typeof s.taskId!=='string'||!count(s.sequence)||typeof s.level!=='number'||!Number.isFinite(s.level)||s.level<0||!count(s.durationMs))throw new SpeechError('protocol');return s as unknown as SpeechMeter;}
export function parsePreferences(v:unknown):RecordingPreferences{const p=object(v);if(!nullableText(p.deviceId)||typeof p.historyEnabled!=='boolean'||typeof p.audioEnabled!=='boolean'||!(p.retentionDays===null||typeof p.retentionDays==='number'&&[7,30,90].includes(p.retentionDays))||!count(p.revision)||p.revision===0)throw new SpeechError('protocol');return p as unknown as RecordingPreferences;}
export function parseDevices(v:unknown):Microphone[]{if(!Array.isArray(v))throw new SpeechError('protocol');return v.map(raw=>{const d=object(raw);if(typeof d.id!=='string'||typeof d.name!=='string'||typeof d.isDefault!=='boolean')throw new SpeechError('protocol');return d as unknown as Microphone;});}
