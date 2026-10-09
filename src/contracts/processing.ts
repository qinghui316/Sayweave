export interface ProcessingSettings {enabled:boolean;revision:number}
export interface ProcessingStatus {settings:ProcessingSettings;available:boolean;model:string|null;error:string|null;personalization?:ProcessingSettings|null}
export interface ProcessingPort {status():Promise<ProcessingStatus>;save(settings:ProcessingSettings):Promise<ProcessingStatus>;saveLanguage?(settings:ProcessingSettings):Promise<ProcessingStatus>;onArchived?(callback:()=>void):Promise<()=>void>}
export function parseProcessing(value:unknown):ProcessingStatus {
 if(!value||typeof value!=='object')throw Error('Invalid processing status');
 const v=value as Record<string,unknown>,s=v.settings;
 if(!s||typeof s!=='object')throw Error('Invalid processing settings');
 const settings=s as Record<string,unknown>;
 if(typeof settings.enabled!=='boolean'||!Number.isSafeInteger(settings.revision)||(settings.revision as number)<1||typeof v.available!=='boolean'||v.model!==null&&typeof v.model!=='string'||v.error!==null&&typeof v.error!=='string')throw Error('Invalid processing status');
 let personalization:ProcessingSettings|null|undefined;
 if(v.personalization!==undefined && v.personalization!==null){if(typeof v.personalization!=='object')throw Error('Invalid personalization settings');const p=v.personalization as Record<string,unknown>;if(typeof p.enabled!=='boolean'||!Number.isSafeInteger(p.revision)||(p.revision as number)<1)throw Error('Invalid personalization settings');personalization={enabled:p.enabled,revision:p.revision as number};}else{personalization=v.personalization;}
 return {settings:{enabled:settings.enabled,revision:settings.revision as number},available:v.available,model:v.model as string|null,error:v.error as string|null,...(personalization!==undefined?{personalization}:{})};
}
