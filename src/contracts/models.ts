export type ModelUsage='speech'|'text';
export const modelErrorCodes=['invalid','notFound','conflict','busy','unavailable','preparing','stopping','credentialMissing','credentialUnavailable','originChanged','forbidden','auth','rateLimit','timeout','network','protocol','cancelled','responseTooLarge','emptyText','service'] as const;
export type ModelErrorCode=typeof modelErrorCodes[number];
export class ModelError extends Error { constructor(readonly code:ModelErrorCode,readonly field?:string,readonly requestId?:string){super(code);} }
export function modelError(e:unknown):ModelError {
 if(e instanceof ModelError)return e;
 if(typeof e==='object'&&e!==null&&'code' in e&&modelErrorCodes.includes(e.code as ModelErrorCode))return new ModelError(e.code as ModelErrorCode,'field' in e&&typeof e.field==='string'?e.field:undefined,'requestId' in e&&typeof e.requestId==='string'?e.requestId:undefined);
 return new ModelError('unavailable');
}
export interface ModelInput {name:string;protocol:'openai-chat';baseUrl:string;modelId:string;usages:ModelUsage[]}
export interface ModelRecord extends ModelInput {id:string;credentialStatus:'present'|'missing'|'unavailable';revision:number;createdAt:number;updatedAt:number}
export interface ModelDefault {usage:ModelUsage;configId:string|null;revision:number}
export interface ModelReplacement {usage:ModelUsage;configId:string|null}
export interface ModelExpectation {defaults:ModelDefault[];replacements:ModelReplacement[]}
export interface ModelOverview {configs:ModelRecord[];defaults:ModelDefault[];cleanupPending:boolean}
export type ModelKeyChange={action:'keep'}|{action:'replace';value:string};
export interface ModelDraft {input:ModelInput;key:ModelKeyChange;id:string|null;revision:number|null}
export interface ModelSave extends ModelDraft {expectation:ModelExpectation|null}
export interface ModelRemove {id:string;revision:number;expectation:ModelExpectation}
export interface ModelDefaultRequest {usage:ModelUsage;configId:string;revision:number}
export interface ModelStatus {state:'preparing'|'ready'|'unavailable'|'stopping';schemaVersion:number;cleanupPending:boolean}
export interface ModelTestResult {testedAt:number;elapsedMs:number}
export interface ModelConfigPort {
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
 return {defaults:view.defaults.map(d=>({...d})),replacements:view.defaults.filter(d=>d.configId===id&&!usages.includes(d.usage)).map(d=>({usage:d.usage,configId:view.configs.find(c=>c.id!==id&&c.usages.includes(d.usage))?.id??null}))};
}
export function validateModelInput(input:ModelInput):ModelInput {
 const field=(raw:string,max:number,key:string)=>{const v=raw.trim();if(!v||[...v].length>max||/[\p{Cc}\u2028\u2029]/u.test(raw))throw new ModelError('invalid',key);return v;};
 const name=field(input.name,64,'name'),modelId=field(input.modelId,256,'modelId');
 let url:URL;try{url=new URL(input.baseUrl.trim());}catch{throw new ModelError('invalid','baseUrl');}
 if(input.baseUrl.trim().length>2048||url.protocol!=='https:'||url.username||url.password||url.search||url.hash||/[\p{Cc}]/u.test(input.baseUrl))throw new ModelError('invalid','baseUrl');
 if(input.protocol!=='openai-chat')throw new ModelError('invalid','protocol');
 if(!input.usages.length||input.usages.length>2||new Set(input.usages).size!==input.usages.length||input.usages.some(u=>!['speech','text'].includes(u)))throw new ModelError('invalid','usages');
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
  if(typeof c.id!=='string'||typeof c.name!=='string'||c.protocol!=='openai-chat'||typeof c.baseUrl!=='string'||typeof c.modelId!=='string'||!Array.isArray(c.usages)||!c.usages.length||c.usages.length>2||!c.usages.every(usage)||new Set(c.usages).size!==c.usages.length||!['present','missing','unavailable'].includes(String(c.credentialStatus))||!num(c.revision,1)||!num(c.createdAt)||!num(c.updatedAt))throw new ModelError('protocol');
  const record={id:c.id,name:c.name,protocol:c.protocol,baseUrl:c.baseUrl,modelId:c.modelId,usages:c.usages,credentialStatus:c.credentialStatus,revision:c.revision,createdAt:c.createdAt,updatedAt:c.updatedAt} as ModelRecord;
  try{validateModelInput(record);}catch{throw new ModelError('protocol');}
  return record;
 });
 const defaults=v.defaults.map(raw=>{const d=obj(raw);if(!usage(d.usage)||!(d.configId===null||typeof d.configId==='string')||!num(d.revision,1))throw new ModelError('protocol');return {usage:d.usage,configId:d.configId,revision:d.revision};});
 if(defaults.length!==2||new Set(defaults.map(d=>d.usage)).size!==2||new Set(configs.map(c=>c.id)).size!==configs.length||defaults.some(d=>d.configId!==null&&!configs.some(c=>c.id===d.configId&&c.usages.includes(d.usage))))throw new ModelError('protocol');
 return {configs,defaults,cleanupPending:v.cleanupPending};
}
export function parseModelStatus(value:unknown):ModelStatus {const v=obj(value);if(!['preparing','ready','unavailable','stopping'].includes(String(v.state))||!num(v.schemaVersion)||typeof v.cleanupPending!=='boolean')throw new ModelError('protocol');return {state:v.state as ModelStatus['state'],schemaVersion:v.schemaVersion,cleanupPending:v.cleanupPending};}
export function parseModelTest(value:unknown):ModelTestResult {const v=obj(value);if(!num(v.testedAt)||!num(v.elapsedMs))throw new ModelError('protocol');return {testedAt:v.testedAt,elapsedMs:v.elapsedMs};}
export function parseModelIds(value:unknown):string[]{if(!Array.isArray(value)||!value.every(v=>typeof v==='string'&&v.length>0&&[...v].length<=256))throw new ModelError('protocol');return [...new Set(value)];}

