import {SpeechError} from './speech';
export interface TranslationLanguage {code:string;label:string;instruction:string}
export interface TranslationSettings {targets:string[];activeTarget:string;revision:number}
export interface TranslationConfiguration {settings:TranslationSettings;languages:TranslationLanguage[]}
export interface TranslationPort {
 read():Promise<TranslationConfiguration>;
 save(config:TranslationSettings):Promise<TranslationConfiguration>;
 select(taskId:string,code:string):Promise<void>;
 menu(taskId:string,open:boolean):Promise<void>;
}
export function parseTranslation(value:unknown):TranslationConfiguration {
 if(!value||typeof value!=='object')throw new SpeechError('protocol');
 const c=value as TranslationConfiguration,s=c.settings;
 if(!s||!Array.isArray(c.languages)||!c.languages.length||!c.languages.every(l=>l&&typeof l.code==='string'&&typeof l.label==='string'&&typeof l.instruction==='string')||new Set(c.languages.map(l=>l.code)).size!==c.languages.length||!Array.isArray(s.targets)||s.targets.length<1||new Set(s.targets).size!==s.targets.length||!s.targets.every(code=>c.languages.some(l=>l.code===code))||!s.targets.includes(s.activeTarget)||!Number.isSafeInteger(s.revision)||s.revision<1)throw new SpeechError('protocol');
 return c;
}
