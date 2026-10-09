import {parseSetting,type ProcessingSettings} from './processing';
export interface LanguageSettings {automaticTerms:ProcessingSettings;personalization:ProcessingSettings}
export interface LearningSettingsPatch {automaticTerms?:ProcessingSettings;personalization?:ProcessingSettings}
export interface ProfileSummary {scope:string;name:string;version:number;preferenceCount:number;observationCount:number;updatedAt:number}
export interface ProfileOverview {preferenceCount:number;applicationCount:number;observationCount:number;shared:ProfileSummary;applications:ProfileSummary[];next:string|null}
export interface ProfileDetails {scope:string;version:number;updatedAt:number;preferences:{rule:string;language:'any'|'han'|'latin'|'mixed'|'other';context:string|null}[]}
export interface LanguageSettingsPort {settings():Promise<LanguageSettings>;save(patch:LearningSettingsPatch):Promise<LanguageSettings>;overview(after?:string|null):Promise<ProfileOverview>;profile(scope:string):Promise<ProfileDetails>;onChanged?(listener:()=>void):Promise<()=>void>}
export function parseLanguageSettings(value:unknown):LanguageSettings {
 if(!value||typeof value!=='object')throw Error('Invalid language settings');
 const v=value as Record<string,unknown>;
 return {automaticTerms:parseSetting(v.automaticTerms),personalization:parseSetting(v.personalization)};
}
const scope=(value:unknown):value is string=>typeof value==='string'&&(value==='shared'||value==='web'||/^app:[a-f0-9]{64}$/.test(value));
const count=(value:unknown):value is number=>Number.isSafeInteger(value)&&(value as number)>=0;
function summary(value:unknown):ProfileSummary {
 if(!value||typeof value!=='object')throw Error('Invalid profile summary');const v=value as Record<string,unknown>;
 if(!scope(v.scope)||typeof v.name!=='string'||v.name.length>256||!count(v.version)||!count(v.preferenceCount)||!count(v.observationCount)||!count(v.updatedAt))throw Error('Invalid profile summary');
 return {scope:v.scope,name:v.name,version:v.version,preferenceCount:v.preferenceCount,observationCount:v.observationCount,updatedAt:v.updatedAt};
}
export function parseProfileOverview(value:unknown):ProfileOverview {
 if(!value||typeof value!=='object')throw Error('Invalid overview');const v=value as Record<string,unknown>;
 if(!count(v.preferenceCount)||!count(v.applicationCount)||!count(v.observationCount)||!Array.isArray(v.applications)||v.applications.length>20||v.next!==null&&!scope(v.next)||v.next==='shared')throw Error('Invalid overview');
 const shared=summary(v.shared),applications=v.applications.map(summary);
 if(shared.scope!=='shared'||applications.some(v=>v.scope==='shared')||new Set(applications.map(v=>v.scope)).size!==applications.length)throw Error('Invalid scopes');
 return {preferenceCount:v.preferenceCount,applicationCount:v.applicationCount,observationCount:v.observationCount,shared,applications,next:v.next as string|null};
}
export function parseProfileDetails(value:unknown):ProfileDetails {
 if(!value||typeof value!=='object')throw Error('Invalid profile');const v=value as Record<string,unknown>;
 if(!scope(v.scope)||!count(v.version)||!count(v.updatedAt)||!Array.isArray(v.preferences)||v.preferences.length>6)throw Error('Invalid profile');
 const preferences=v.preferences.map(value=>{if(!value||typeof value!=='object')throw Error('Invalid preference');const p=value as Record<string,unknown>;if(typeof p.rule!=='string'||p.rule.length>512||!['any','han','latin','mixed','other'].includes(String(p.language))||p.context!==null&&typeof p.context!=='string')throw Error('Invalid preference');return {rule:p.rule,language:p.language as ProfileDetails['preferences'][number]['language'],context:p.context as string|null};});
 return {scope:v.scope,version:v.version,updatedAt:v.updatedAt,preferences};
}
