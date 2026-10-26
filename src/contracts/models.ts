import {localAsrErrorCodes} from './local-asr';
export type ModelUsage='speech'|'text';
export const modelErrorCodes=[...localAsrErrorCodes,'invalid','notFound','conflict','busy','unavailable','preparing','stopping','credentialMissing','credentialUnavailable','originChanged','forbidden','auth','rateLimit','timeout','network','protocol','cancelled','responseTooLarge','emptyText','service'] as const;
export type ModelErrorCode=typeof modelErrorCodes[number];
export class ModelError extends Error { constructor(readonly code:ModelErrorCode,readonly field?:string,readonly requestId?:string){super(code);} }
export function modelError(e:unknown):ModelError {
 if(e instanceof ModelError)return e;
 if(typeof e==='object'&&e!==null&&'code' in e&&modelErrorCodes.includes(e.code as ModelErrorCode))return new ModelError(e.code as ModelErrorCode,'field' in e&&typeof e.field==='string'?e.field:undefined,'requestId' in e&&typeof e.requestId==='string'?e.requestId:undefined);
 return new ModelError('unavailable');
}
export const modelProtocols = {
 'openai-chat': {usages:['text'], liveText:false, textTest:true, originOnly:false},
 'qwen-audio-streaming': {usages:['speech'], liveText:true, textTest:false, originOnly:true},
 'doubao-asr-streaming': {usages:['speech'], liveText:true, textTest:false, originOnly:true},
} as const;
export type ModelProtocol=keyof typeof modelProtocols;
export type ModelHelp='qwen-api-key'|'doubao-api-key';
export function isModelProtocol(v:unknown):v is ModelProtocol {return typeof v==='string' && Object.hasOwn(modelProtocols,v);}
export interface ModelInput {name:string;protocol:ModelProtocol;baseUrl:string;modelId:string;usages:ModelUsage[]}
export function modelExecutable(c:ModelInput|LocalModelInput,u:ModelUsage){return 'assetId' in c?u==='speech':c.usages.includes(u)&&(modelProtocols[c.protocol].usages as readonly ModelUsage[]).includes(u);}
export interface ModelRecord extends ModelInput {id:string;credentialStatus:'present'|'missing'|'unavailable';revision:number;createdAt:number;updatedAt:number}
export interface ModelDefault {usage:ModelUsage;configId:string|null;revision:number}
export interface ModelReplacement {usage:ModelUsage;configId:string|null}
export interface ModelExpectation {defaults:ModelDefault[];replacements:ModelReplacement[]}
export interface ModelOverview {configs:ModelRecord[];defaults:ModelDefault[];cleanupPending:boolean}
export const localAssetId='nemotron-3.5-asr-streaming-0.6b-q4';
export interface LocalModelInput {name:string;assetId:string;engine:string;device:'auto'|'cpu'}
export interface LocalModelRecord extends LocalModelInput {id:string;revision:number;createdAt:number;updatedAt:number}
export interface LocalModelSave {input:LocalModelInput;id:string|null;revision:number|null}
export interface LocalAssetStatus {assetId:string;state:'missing'|'available'|'downloading'|'verifying'|'ready'|'failed';received:number;bytes:number;error:string|null}
export interface LocalModelPort {
 status(assetId:string):Promise<LocalAssetStatus>;
 download(assetId:string):Promise<LocalAssetStatus>;
 cancel(assetId:string):Promise<void>;
 delete(assetId:string):Promise<void>;
 save(request:LocalModelSave):Promise<ModelOverview>;
}
export interface ModelOverview {localConfigs?:LocalModelRecord[]}
export type SpeechModelRecord=ModelRecord|LocalModelRecord;
export function defaultSpeechModel(view:ModelOverview):SpeechModelRecord|undefined {
 const id=view.defaults.find(d=>d.usage==='speech')?.configId;
 return view.configs.find(c=>c.id===id)??view.localConfigs?.find(c=>c.id===id);
}
export type ModelKeyChange={action:'keep'}|{action:'replace';value:string};
export interface ModelDraft {input:ModelInput;key:ModelKeyChange;id:string|null;revision:number|null}
export interface ModelSave extends ModelDraft {expectation:ModelExpectation|null}
export interface ModelRemove {id:string;revision:number;expectation:ModelExpectation}
export interface ModelDefaultRequest {usage:ModelUsage;configId:string;revision:number}
export interface ModelStatus {state:'preparing'|'ready'|'unavailable'|'stopping';schemaVersion:number;cleanupPending:boolean}
export interface ModelTestResult {testedAt:number;elapsedMs:number}
export interface ModelConfigPort {
 readonly local?:LocalModelPort;
 readonly mode:'native'|'simulation';
 status(retry?:boolean):Promise<ModelStatus>;
 overview():Promise<ModelOverview>;
 save(request:ModelSave):Promise<ModelOverview>;
 remove(request:ModelRemove):Promise<ModelOverview>;
 setDefault(request:ModelDefaultRequest):Promise<ModelOverview>;
 fetchModels(draft:ModelDraft,requestId:string):Promise<string[]>;
 testTextModel(draft:ModelDraft,requestId:string):Promise<ModelTestResult>;
 cancelRequest(requestId:string):Promise<void>;
}
export function modelExpectation(view:ModelOverview,id:string,usages:ModelUsage[]):ModelExpectation {
 return {defaults:view.defaults.map(d=>({...d})),replacements:view.defaults.filter(d=>d.configId===id&&!usages.includes(d.usage)).map(d=>({usage:d.usage,configId:[...view.configs.filter(c=>c.id!==id&&modelExecutable(c,d.usage)),...(d.usage==='speech'?(view.localConfigs??[]).filter(c=>c.id!==id):[])].sort((a,b)=>a.createdAt-b.createdAt||a.id.localeCompare(b.id))[0]?.id??null}))};
}
export function validateModelInput(input:ModelInput):ModelInput {
 const field=(raw:string,max:number,key:string)=>{const v=raw.trim();if(!v||[...v].length>max||/[\p{Cc}\u2028\u2029]/u.test(raw))throw new ModelError('invalid',key);return v;};
 const name=field(input.name,64,'name'),modelId=field(input.modelId,256,'modelId');
 let url:URL;try{url=new URL(input.baseUrl.trim());}catch{throw new ModelError('invalid','baseUrl');}
 if(input.baseUrl.trim().length>2048||url.protocol!=='https:'||url.username||url.password||url.search||url.hash||/[\p{Cc}]/u.test(input.baseUrl))throw new ModelError('invalid','baseUrl');
 if(!isModelProtocol(input.protocol))throw new ModelError('invalid','protocol');
 if(!input.usages.length||input.usages.length>2||new Set(input.usages).size!==input.usages.length||input.usages.some(u=>!['speech','text'].includes(u)))throw new ModelError('invalid','usages');
 if(modelProtocols[input.protocol].originOnly&&(input.usages.length!==1||input.usages[0]!=='speech'))throw new ModelError('invalid','usages');
 if(modelProtocols[input.protocol].originOnly&&url.pathname!=='/')throw new ModelError('invalid','baseUrl');
 if(input.protocol==='doubao-asr-streaming'&&modelId!=='Doubao-Seed-ASR-2.0')throw new ModelError('invalid','modelId');
 return {...input,name,modelId,baseUrl:url.href.replace(/\/+$/u,''),usages:[...input.usages].sort()};
}
export function modelSameOrigin(a:string,b:string){try{return new URL(a).origin===new URL(b).origin;}catch{return false;}}
export function validateModelKey(value:string){const key=value.trim();if(!key||new TextEncoder().encode(key).length>1024||!/^[\x21-\x7e]+$/u.test(key))throw new ModelError('invalid','key');return key;}
const obj=(v:unknown):Record<string,unknown>=>{if(typeof v!=='object'||v===null||Array.isArray(v))throw new ModelError('protocol');return v as Record<string,unknown>;};
const num=(v:unknown,min=0):v is number=>typeof v==='number'&&Number.isSafeInteger(v)&&v>=min;
const usage=(v:unknown):v is ModelUsage=>v==='speech'||v==='text';
export function parseModelOverview(value:unknown):ModelOverview {
 const v=obj(value);if(!Array.isArray(v.configs)||!Array.isArray(v.defaults)||typeof v.cleanupPending!=='boolean')throw new ModelError('protocol');
 const configs=v.configs.map(raw=>{const c=obj(raw);
  if(typeof c.id!=='string'||typeof c.name!=='string'||!isModelProtocol(c.protocol)||typeof c.baseUrl!=='string'||typeof c.modelId!=='string'||!Array.isArray(c.usages)||!c.usages.length||c.usages.length>2||!c.usages.every(usage)||new Set(c.usages).size!==c.usages.length||!['present','missing','unavailable'].includes(String(c.credentialStatus))||!num(c.revision,1)||!num(c.createdAt)||!num(c.updatedAt))throw new ModelError('protocol');
  const record={id:c.id,name:c.name,protocol:c.protocol,baseUrl:c.baseUrl,modelId:c.modelId,usages:c.usages,credentialStatus:c.credentialStatus,revision:c.revision,createdAt:c.createdAt,updatedAt:c.updatedAt} as ModelRecord;
  try{validateModelInput(record);}catch{throw new ModelError('protocol');}
  return record;
 });
 const defaults=v.defaults.map(raw=>{const d=obj(raw);if(!usage(d.usage)||!(d.configId===null||typeof d.configId==='string')||!num(d.revision,1))throw new ModelError('protocol');return {usage:d.usage,configId:d.configId,revision:d.revision};});
 if(v.localConfigs!==undefined&&!Array.isArray(v.localConfigs))throw new ModelError('protocol');
 const localConfigs:unknown[]=Array.isArray(v.localConfigs)?v.localConfigs:[];
 const local=localConfigs.map(raw=>{const c=obj(raw);if(typeof c.id!=='string'||typeof c.name!=='string'||typeof c.assetId!=='string'||c.engine!=='transcribe-cpp-0.1.3'||!['auto','cpu'].includes(String(c.device))||!num(c.revision,1)||!num(c.createdAt)||!num(c.updatedAt))throw new ModelError('protocol');return {id:c.id,name:c.name,assetId:c.assetId,engine:c.engine,device:c.device,revision:c.revision,createdAt:c.createdAt,updatedAt:c.updatedAt} as LocalModelRecord;});
 const ids=[...configs,...local].map(c=>c.id);
 if(defaults.length!==2||new Set(defaults.map(d=>d.usage)).size!==2||new Set(ids).size!==ids.length||defaults.some(d=>d.configId!==null&&!configs.some(c=>c.id===d.configId&&c.usages.includes(d.usage))&&!(d.usage==='speech'&&local.some(c=>c.id===d.configId))))throw new ModelError('protocol');
 return {configs,localConfigs:local,defaults,cleanupPending:v.cleanupPending};
}
export function parseLocalAsset(value:unknown):LocalAssetStatus {const v=obj(value);if(typeof v.assetId!=='string'||!['missing','available','downloading','verifying','ready','failed'].includes(String(v.state))||!num(v.received)||!num(v.bytes,1)||v.received>v.bytes||!(v.error===null||typeof v.error==='string'))throw new ModelError('protocol');return v as unknown as LocalAssetStatus;}
export function parseModelStatus(value:unknown):ModelStatus {const v=obj(value);if(!['preparing','ready','unavailable','stopping'].includes(String(v.state))||!num(v.schemaVersion)||typeof v.cleanupPending!=='boolean')throw new ModelError('protocol');return {state:v.state as ModelStatus['state'],schemaVersion:v.schemaVersion,cleanupPending:v.cleanupPending};}
export function parseModelTest(value:unknown):ModelTestResult {const v=obj(value);if(!num(v.testedAt)||!num(v.elapsedMs))throw new ModelError('protocol');return {testedAt:v.testedAt,elapsedMs:v.elapsedMs};}
export function parseModelIds(value:unknown):string[]{if(!Array.isArray(value)||!value.every(v=>typeof v==='string'&&v.length>0&&[...v].length<=256))throw new ModelError('protocol');return [...new Set(value)];}
