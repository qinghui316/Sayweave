export interface ProcessingSettings {enabled:boolean;revision:number}
export interface ProcessingStatus {settings:ProcessingSettings;available:boolean;model:string|null;error:string|null}
export interface ProcessingPort {settings():Promise<ProcessingSettings>;status():Promise<ProcessingStatus>;save(settings:ProcessingSettings):Promise<ProcessingSettings>;onArchived?(callback:()=>void):Promise<()=>void>}
export function parseSetting(value:unknown):ProcessingSettings {
 if(!value||typeof value!=='object')throw Error('Invalid settings');
 const v=value as Record<string,unknown>;
 if(typeof v.enabled!=='boolean'||!Number.isSafeInteger(v.revision)||(v.revision as number)<1)throw Error('Invalid settings');
 return {enabled:v.enabled,revision:v.revision as number};
}
export function parseProcessing(value:unknown):ProcessingStatus {
 if(!value||typeof value!=='object')throw Error('Invalid processing status');
 const v=value as Record<string,unknown>;
 if(typeof v.available!=='boolean'||v.model!==null&&typeof v.model!=='string'||v.error!==null&&typeof v.error!=='string')throw Error('Invalid processing status');
 return {settings:parseSetting(v.settings),available:v.available,model:v.model as string|null,error:v.error as string|null};
}
